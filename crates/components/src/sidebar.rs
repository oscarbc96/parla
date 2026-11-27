use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use crate::conversation_menu::level_mark;
use domain::{
    Conversation, ConversationId, ConversationKind, DIRECT_MESSAGES_SHOWN, Member, NotifyLevel,
    WorkspaceState,
};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, KeyBinding, MouseButton,
    MouseDownEvent, Render, SharedString, UniformListScrollHandle, Window, actions, div,
    prelude::*, px, uniform_list,
};
use i18n::Localizer;
use settings::{SettingsStore, SidebarSettings, SidebarSort};
use themes::ActiveTheme;
use themes::parla_tokens;

actions!(sidebar, [SelectNext, SelectPrev, Open, NextUnread]);

/// Key context of the sidebar view.
pub const KEY_CONTEXT: &str = "Sidebar";

/// Key context of a text field inside the sidebar (a section name): the list
/// keys leave it alone, so Space types and Enter commits the name.
pub const FIELD_KEY_CONTEXT: &str = "SidebarField";

/// The sidebar's list keys, outside its text fields.
const LIST_KEYS: &str = "Sidebar && !SidebarField";

/// Registers the default sidebar key bindings.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, Some(LIST_KEYS)),
        KeyBinding::new("up", SelectPrev, Some(LIST_KEYS)),
        KeyBinding::new("enter", Open, Some(LIST_KEYS)),
        KeyBinding::new("space", Open, Some(LIST_KEYS)),
    ]);
}

/// A sidebar option the settings popover changes (the same settings as the
/// Appearance page, applied live).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarOption {
    ShowUnreads(bool),
    Sort(SidebarSort),
    Density(settings::Density),
}

/// A conversation row picked up in organise mode.
#[derive(Clone, Debug, PartialEq)]
pub struct DraggedRow {
    pub conversation: ConversationId,
    pub label: SharedString,
}

/// A custom section header picked up in organise mode.
#[derive(Clone, Debug, PartialEq)]
pub struct DraggedSection {
    pub id: u32,
    pub label: SharedString,
}

/// What follows the pointer while a row or section is dragged.
struct DragPreview(SharedString);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = parla_tokens(cx.theme());
        div()
            .h(themes::metrics::LAYOUT_ROW_COMPACT)
            .px(themes::metrics::SPACE_5)
            .flex()
            .items_center()
            .rounded(themes::metrics::RADIUS_SM)
            .bg(tokens.sidebar_active)
            .text_color(tokens.sidebar_text_strong)
            .text_size(themes::metrics::FONT_SIZE_MD)
            .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
            .shadow_md()
            .child(self.0.clone())
    }
}

/// A titled group of sidebar rows the user can collapse (per workspace).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SidebarSection {
    Starred,
    Unreads,
    /// One of the user's own sections (`sidebar.sections`), by its id.
    Custom(u32),
    Channels,
    DirectMessages,
}

impl SidebarSection {
    /// The built-in sections (custom ones come from the settings).
    pub const ALL: &'static [SidebarSection] = &[
        SidebarSection::Starred,
        SidebarSection::Unreads,
        SidebarSection::Channels,
        SidebarSection::DirectMessages,
    ];

    /// Whether the user can fold it. Unreads cannot: folding it would hide
    /// exactly what it is for.
    pub fn collapsible(self) -> bool {
        self != SidebarSection::Unreads
    }

    /// The header's test selector.
    pub fn selector(self) -> Cow<'static, str> {
        match self {
            SidebarSection::Starred => "sidebar-section-starred".into(),
            SidebarSection::Unreads => "sidebar-section-unreads".into(),
            SidebarSection::Custom(id) => format!("sidebar-section-custom-{id}").into(),
            SidebarSection::Channels => "sidebar-section-channels".into(),
            SidebarSection::DirectMessages => "sidebar-section-direct_messages".into(),
        }
    }

    /// The stable name the store keeps for a collapsed section.
    pub fn key(self) -> Cow<'static, str> {
        match self {
            SidebarSection::Starred => "starred".into(),
            SidebarSection::Unreads => "unreads".into(),
            SidebarSection::Custom(id) => format!("section:{id}").into(),
            SidebarSection::Channels => "channels".into(),
            SidebarSection::DirectMessages => "direct_messages".into(),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        if let Some(id) = key.strip_prefix("section:") {
            return id.parse().ok().map(SidebarSection::Custom);
        }
        Self::ALL.iter().copied().find(|s| s.key() == key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SidebarRow {
    /// A section title: clicking it collapses or expands the section.
    Header {
        section: SidebarSection,
        label: SharedString,
        collapsed: bool,
    },
    Conversation {
        id: ConversationId,
        label: SharedString,
        kind: ConversationKind,
        unread: bool,
        mentions: u32,
        selected: bool,
        /// Muted conversations are dimmed and never listed under "Unreads".
        muted: bool,
        /// A draft is saved for the conversation.
        draft: bool,
        /// The notification level, when not the default (the row shows a mark).
        level: NotifyLevel,
        /// A direct message's peer is active (`Some(true)`) or away; `None`
        /// for other kinds and while unknown (no dot).
        presence: Option<bool>,
        /// The row reserves room for the dot (a direct message of a provider
        /// that reports presence), so names line up while it is unknown.
        dot_slot: bool,
        /// A huddle is live here: the number of people in it.
        huddle: Option<u32>,
    },
    /// Ends the "Direct messages" section when it holds more than
    /// [`DIRECT_MESSAGES_SHOWN`]: expands the rest, or collapses it again.
    ShowMore {
        label: SharedString,
        /// Rows the collapsed section leaves out.
        hidden: usize,
        expanded: bool,
    },
}

/// What the sidebar needs from the session, so `components` does not depend on `parla`.
pub trait SidebarSource {
    fn rows(&self, cx: &App) -> Vec<SidebarRow>;
    fn open(&mut self, id: ConversationId, cx: &mut App);
    /// Expands or collapses the "Direct messages" section (the "Show more" row).
    fn toggle_direct_messages(&mut self, _cx: &mut App) {}
    /// Collapses or expands a section (its header was clicked).
    fn toggle_section(&mut self, _section: SidebarSection, _cx: &mut App) {}
    /// The "Drafts" entry: `None` hides it (no drafts), `Some(n)` shows it
    /// with the number of saved drafts.
    fn drafts_entry(&self) -> Option<usize> {
        None
    }
    /// Opens the workspace's Drafts view.
    fn open_drafts(&mut self, _cx: &mut App) {}
    /// The "People" entry is shown (the host has a People directory).
    fn people_entry(&self) -> bool {
        false
    }
    /// Opens the workspace's People directory.
    fn open_people(&mut self, _cx: &mut App) {}
    /// The workspace on screen; shown at the top of the sidebar when present.
    fn workspace_name(&self, _cx: &App) -> Option<SharedString> {
        None
    }
    /// The "Scheduled" entry above the list: `None` hides it, `Some(n)` shows it
    /// with the number of scheduled messages.
    fn scheduled_entry(&self) -> Option<usize> {
        None
    }
    /// Opens the workspace's Scheduled view.
    fn open_scheduled(&mut self, _cx: &mut App) {}
    /// The "Reminders" entry above the list: `None` hides it, `Some(n)` shows it
    /// with the number of reminders still to do.
    fn reminders_entry(&self) -> Option<usize> {
        None
    }
    /// Opens the workspace's Reminders view.
    fn open_reminders(&mut self, _cx: &mut App) {}
    /// The "Threads" entry: `None` hides it (the provider has no thread
    /// inbox), `Some(n)` shows it with `n` unread replies (0 shows no count).
    fn threads_entry(&self) -> Option<usize> {
        None
    }
    /// Opens the workspace's Threads view.
    fn open_threads(&mut self, _cx: &mut App) {}
    /// The "Later" entry: `None` hides it (the provider has no saved
    /// items), `Some(n)` shows it with `n` open items (0 shows no count).
    fn later_entry(&self) -> Option<usize> {
        None
    }
    /// Opens the workspace's Later view.
    fn open_later(&mut self, _cx: &mut App) {}
    /// "Notifications paused until ..." while the user's own pause is on.
    fn paused_label(&self, _cx: &App) -> Option<String> {
        None
    }
    /// Ends the pause (the strip's Resume).
    fn resume_notifications(&mut self, _cx: &mut App) {}
    /// Edits the user's own sections (create, rename, delete, reorder, move a
    /// conversation); the host keeps them in the workspace's settings file.
    fn section_op(&mut self, _op: crate::SectionOp, _cx: &mut App) {}
    /// The settings popover changed a sidebar option.
    fn set_sidebar_option(&mut self, _option: SidebarOption, _cx: &mut App) {}
    /// Show Unreads and sort as this workspace uses them, for the popover;
    /// the app-wide settings by default.
    fn sidebar_options(&self, cx: &App) -> (bool, SidebarSort) {
        sidebar_options(cx)
    }
}

/// The sidebar asks its host for something it does not own.
#[derive(Clone, Debug, PartialEq)]
pub enum SidebarEvent {
    /// Right click on a conversation row: the host opens the notification menu.
    ContextMenu {
        conversation: ConversationId,
        position: gpui::Point<gpui::Pixels>,
    },
    /// The "Jump or search" field: the host opens the quick switcher.
    QuickSwitch,
    /// The New message button: the host asks where to write, then focuses
    /// that composer.
    NewMessage,
    /// The Channels "+": the host lists the channels to open.
    BrowseChannels,
    /// The Direct messages "+": the host lists the people to message.
    BrowsePeople,
    /// The workspace name: the host opens the workspace menu at `position`.
    WorkspaceMenu { position: gpui::Point<gpui::Pixels> },
}

/// Handle to display-name lookup for group labels, built once per member
/// update instead of scanning every member for every participant of every row.
#[derive(Default, Debug, Clone)]
pub struct MemberIndex {
    /// Handle to display name (members with an empty display name are left out).
    names: HashMap<Arc<str>, Arc<str>>,
    /// The signed-in user's handle, left out of group labels.
    me: Option<Arc<str>>,
}

/// The one name rule for people: display name, else real name, else handle.
/// `None` only when the member has none of them.
pub fn best_name(m: &Member) -> Option<Arc<str>> {
    [&m.display_name, &m.real_name, &m.handle]
        .into_iter()
        .find(|n| !n.is_empty())
        .cloned()
}

impl MemberIndex {
    pub fn build(state: &WorkspaceState) -> Self {
        let me = state
            .self_user
            .as_ref()
            .and_then(|u| state.members.get(u))
            .map(|m| m.handle.clone())
            .filter(|h| !h.is_empty());
        let names = state
            .members
            .values()
            .filter(|m| !m.handle.is_empty())
            .filter_map(|m| best_name(m).map(|n| (m.handle.clone(), n)))
            .collect();
        Self { names, me }
    }
}

impl MemberIndex {
    /// Applies one member update in place. Returns true when a label can change
    /// (display name or handle changed, or it is the signed-in user), so
    /// profile and presence updates that touch neither leave the rows alone.
    pub fn update(&mut self, old: Option<&Member>, new: &Member, is_self: bool) -> bool {
        let before = old.map(|m| (&m.handle, &m.display_name, &m.real_name));
        if before == Some((&new.handle, &new.display_name, &new.real_name)) && !is_self {
            return false;
        }
        if let Some(o) = old
            && !o.handle.is_empty()
        {
            self.names.remove(&o.handle);
        }
        if !new.handle.is_empty()
            && let Some(n) = best_name(new)
        {
            self.names.insert(new.handle.clone(), n);
        }
        if is_self {
            self.me = Some(new.handle.clone()).filter(|h| !h.is_empty());
        }
        true
    }
}

/// A group's other participants as a comma-separated list of display names.
/// A participant whose member is not loaded yet shows its handle (never a raw
/// id), and the label updates when the member arrives. The signed-in user is left out.
fn group_label(index: &MemberIndex, participants: &[Arc<str>]) -> Arc<str> {
    let names: Vec<&str> = participants
        .iter()
        .filter(|h| Some(*h) != index.me.as_ref())
        .map(|h| match index.names.get(h) {
            Some(n) => &**n,
            None => &**h,
        })
        .collect();
    names.join(", ").into()
}

/// A group's other members by id, as a comma-separated list of names. `None`
/// when the provider gave no ids or none of them is loaded yet (the handles in
/// the group's name serve until then). Members still being looked up are left
/// out and join the label when they arrive.
pub(crate) fn group_member_names(state: &WorkspaceState, c: &Conversation) -> Option<Arc<str>> {
    let names: Vec<Arc<str>> = c
        .member_ids
        .iter()
        .filter(|u| state.self_user.as_ref() != Some(*u))
        .filter_map(|u| state.members.get(u).and_then(best_name))
        .collect();
    (!names.is_empty()).then(|| names.join(", ").into())
}

/// The display name of a conversation: group participants, a DM's peer, else
/// the conversation name. A DM whose peer is not resolved reads as the neutral
/// "Unknown" placeholder, never as the raw provider id.
pub fn conversation_label(
    state: &WorkspaceState,
    index: &MemberIndex,
    c: &Conversation,
    l: &Localizer,
) -> Arc<str> {
    // Members by id: a member with no handle (or one that differs from the
    // handle in the group's internal name) still gets its name.
    if matches!(c.kind, ConversationKind::Group)
        && let Some(names) = group_member_names(state, c)
    {
        return names;
    }
    match (&c.kind, &c.peer) {
        (ConversationKind::Group, _) if !c.participants.is_empty() => {
            group_label(index, &c.participants)
        }
        (ConversationKind::Direct, Some(p)) => state
            .members
            .get(p)
            .and_then(best_name)
            .unwrap_or_else(|| Arc::from(t!(l, "author-unknown").into_owned())),
        (ConversationKind::Direct | ConversationKind::Group, _) if c.name.is_empty() => {
            Arc::from(t!(l, "author-unknown").into_owned())
        }
        _ => c.name.clone(),
    }
}

/// The mute flag in force: the user's own (provider or Parla), else the conversation's.
fn is_muted(state: &WorkspaceState, c: &Conversation) -> bool {
    // A timed mute shows the mark until its timer ends it.
    state.prefs_for(&c.id).map_or(c.is_muted, |p| p.muted)
        || state
            .local_notify_extras
            .get(&c.id)
            .is_some_and(|e| e.mute_until.is_some())
}

/// The small presence mark: a filled dot when active, a hollow ring when away
/// (shape as well as colour, so it reads without telling the colours apart).
pub fn presence_dot(
    id: impl Into<gpui::ElementId>,
    active: bool,
    active_color: gpui::Hsla,
    away_color: gpui::Hsla,
) -> gpui::Stateful<gpui::Div> {
    let id: gpui::ElementId = id.into();
    let selector = format!("{id}-{}", if active { "active" } else { "away" });
    let d = div()
        .id(id)
        .debug_selector(move || selector)
        .flex_none()
        .size(px(8.0))
        .rounded_full();
    if active {
        d.bg(active_color)
    } else {
        d.border_2().border_color(away_color)
    }
}

fn row(
    state: &WorkspaceState,
    index: &MemberIndex,
    c: &Conversation,
    selected: Option<&ConversationId>,
    drafts: &HashSet<ConversationId>,
    l: &Localizer,
) -> SidebarRow {
    let label = conversation_label(state, index, c, l);
    let prefs = state.prefs_for(&c.id);
    SidebarRow::Conversation {
        id: c.id.clone(),
        label: SharedString::from(label.to_string()),
        kind: c.kind,
        unread: c.unread,
        mentions: c.mentions,
        selected: selected == Some(&c.id),
        muted: is_muted(state, c),
        draft: drafts.contains(&c.id),
        level: prefs.map(|p| p.level).unwrap_or_default(),
        presence: crate::header::dm_presence(state, c),
        dot_slot: state.caps.presence_watch && c.kind == ConversationKind::Direct,
        huddle: state
            .huddles
            .get(&c.id)
            .map(|h| u32::try_from(h.participants.len()).unwrap_or(u32::MAX)),
    }
}

/// Sections "Starred" (the user's starred conversations, by name; hidden when
/// none), "Unreads" (when enabled: unread, non-muted conversations, mentions
/// first, then latest activity), "Channels" (member channels, alphabetical or
/// by latest activity per `settings.sort`) and "Direct messages" (DMs and
/// groups, latest activity first). Archived conversations and closed DMs are
/// hidden. Starred conversations leave "Channels" and "Direct messages"; rows
/// in "Unreads" also appear in their own section.
pub fn build_rows(
    state: &WorkspaceState,
    selected: Option<&ConversationId>,
    settings: &SidebarSettings,
    drafts: &HashSet<ConversationId>,
    l: &Localizer,
) -> Vec<SidebarRow> {
    build_rows_indexed(
        state,
        &MemberIndex::build(state),
        selected,
        settings,
        drafts,
        l,
    )
}

/// [`build_rows`] with a prebuilt [`MemberIndex`], for callers that cache it.
pub fn build_rows_indexed(
    state: &WorkspaceState,
    index: &MemberIndex,
    selected: Option<&ConversationId>,
    settings: &SidebarSettings,
    drafts: &HashSet<ConversationId>,
    l: &Localizer,
) -> Vec<SidebarRow> {
    build_rows_pinned(state, index, selected, None, settings, drafts, l)
}

/// [`build_rows_indexed`] that also keeps `pinned` (the conversation that was
/// unread when opened) in "Unreads" while it stays open, so opening a
/// conversation does not make the list jump under the cursor.
pub fn build_rows_pinned(
    state: &WorkspaceState,
    index: &MemberIndex,
    selected: Option<&ConversationId>,
    pinned: Option<&ConversationId>,
    settings: &SidebarSettings,
    drafts: &HashSet<ConversationId>,
    l: &Localizer,
) -> Vec<SidebarRow> {
    build_rows_with(state, index, selected, pinned, settings, drafts, l, false)
}

/// [`build_rows_pinned`] with the "Direct messages" section expanded
/// (`dms_expanded`) or collapsed to the [`DIRECT_MESSAGES_SHOWN`] most recent
/// plus unread and open ones.
#[allow(clippy::too_many_arguments)]
pub fn build_rows_with(
    state: &WorkspaceState,
    index: &MemberIndex,
    selected: Option<&ConversationId>,
    pinned: Option<&ConversationId>,
    settings: &SidebarSettings,
    drafts: &HashSet<ConversationId>,
    l: &Localizer,
    dms_expanded: bool,
) -> Vec<SidebarRow> {
    build_rows_sections(
        state,
        index,
        selected,
        pinned,
        settings,
        drafts,
        l,
        dms_expanded,
        &BTreeSet::new(),
    )
}

/// [`build_rows_with`] with the `collapsed` sections folded: a collapsed
/// section keeps its header, its unread rows and the selected one.
#[allow(clippy::too_many_arguments)]
pub fn build_rows_sections(
    state: &WorkspaceState,
    index: &MemberIndex,
    selected: Option<&ConversationId>,
    pinned: Option<&ConversationId>,
    settings: &SidebarSettings,
    drafts: &HashSet<ConversationId>,
    l: &Localizer,
    dms_expanded: bool,
    collapsed: &BTreeSet<SidebarSection>,
) -> Vec<SidebarRow> {
    let visible = || {
        state
            .conversations
            .values()
            .filter(|c| c.is_member && !c.is_archived)
    };
    // Starred conversations leave their own section for "Starred" (the user's
    // own list, kept by Parla), listed by name.
    let mut starred: Vec<&Conversation> = visible()
        .filter(|c| state.starred.contains(&c.id))
        // Closed DMs stay hidden here too, except the one on screen.
        .filter(|c| {
            !matches!(c.kind, ConversationKind::Direct | ConversationKind::Group)
                || c.is_open
                || selected == Some(&c.id)
        })
        .collect();
    starred.sort_by_cached_key(|c| {
        (
            conversation_label(state, index, c, l).to_lowercase(),
            c.id.clone(),
        )
    });
    // The user's own sections claim their conversations (the first section
    // wins a conversation listed twice); a starred one stays in Starred.
    let mut claimed: HashMap<&str, usize> = HashMap::new();
    for (i, sec) in settings.sections.iter().enumerate() {
        for c in &sec.conversations {
            claimed.entry(c.as_str()).or_insert(i);
        }
    }
    let mut customs: Vec<Vec<&Conversation>> = vec![Vec::new(); settings.sections.len()];
    if !claimed.is_empty() {
        for c in visible().filter(|c| !state.starred.contains(&c.id)) {
            let Some(&i) = claimed.get(&*c.id.0) else {
                continue;
            };
            // Closed DMs stay hidden here too, except the one on screen.
            if matches!(c.kind, ConversationKind::Direct | ConversationKind::Group)
                && !c.is_open
                && selected != Some(&c.id)
            {
                continue;
            }
            customs[i].push(c);
        }
        for list in &mut customs {
            match settings.sort {
                SidebarSort::Alphabetical => list.sort_by_cached_key(|c| {
                    (
                        conversation_label(state, index, c, l).to_lowercase(),
                        c.id.clone(),
                    )
                }),
                SidebarSort::Recent => {
                    list.sort_by(|a, b| b.latest.cmp(&a.latest).then_with(|| a.id.cmp(&b.id)))
                }
            }
        }
    }
    let mut channels: Vec<&Conversation> = visible()
        .filter(|c| !state.starred.contains(&c.id))
        .filter(|c| !claimed.contains_key(&*c.id.0))
        .filter(|c| {
            matches!(
                c.kind,
                ConversationKind::Channel | ConversationKind::PrivateChannel
            )
        })
        .collect();
    match settings.sort {
        SidebarSort::Alphabetical => {
            channels.sort_by_cached_key(|c| (c.name.to_lowercase(), c.id.clone()))
        }
        SidebarSort::Recent => channels.sort_by_cached_key(|c| {
            (
                std::cmp::Reverse(c.latest.clone()),
                c.name.to_lowercase(),
                c.id.clone(),
            )
        }),
    }
    // Closed DMs stay hidden (until a new message opens them), except the one
    // on screen.
    let mut dms: Vec<&Conversation> = visible()
        .filter(|c| !state.starred.contains(&c.id))
        .filter(|c| !claimed.contains_key(&*c.id.0))
        .filter(|c| matches!(c.kind, ConversationKind::Direct | ConversationKind::Group))
        .filter(|c| c.is_open || selected == Some(&c.id))
        .collect();
    dms.sort_by(|a, b| b.latest.cmp(&a.latest).then_with(|| a.id.cmp(&b.id)));
    let total_dms = dms.len();
    if !dms_expanded && total_dms > DIRECT_MESSAGES_SHOWN {
        // The most recent stay, and so do unread ones and the open one.
        let mut rank = 0;
        dms.retain(|c| {
            rank += 1;
            rank <= DIRECT_MESSAGES_SHOWN || c.unread || c.mentions > 0 || selected == Some(&c.id)
        });
    }
    let hidden_dms = total_dms - dms.len();

    let mut unreads: Vec<&Conversation> = if settings.show_unreads_section {
        visible()
            .filter(|c| c.is_open || selected == Some(&c.id))
            .filter(|c| {
                (c.unread && !is_muted(state, c)) || (pinned == Some(&c.id) && selected == pinned)
            })
            .collect()
    } else {
        Vec::new()
    };
    unreads.sort_by(|a, b| {
        (b.mentions > 0)
            .cmp(&(a.mentions > 0))
            .then_with(|| b.latest.cmp(&a.latest))
            .then_with(|| a.id.cmp(&b.id))
    });

    let custom_rows: usize = customs.iter().map(|c| c.len() + 1).sum();
    let mut rows = Vec::with_capacity(
        starred.len() + unreads.len() + custom_rows + channels.len() + dms.len() + 4,
    );
    // A collapsed section keeps what still asks for attention: unread rows
    // and the conversation on screen.
    let keep = |c: &&Conversation| {
        selected == Some(&c.id) || (c.unread && !is_muted(state, c)) || c.mentions > 0
    };
    let section = |rows: &mut Vec<SidebarRow>,
                   s: SidebarSection,
                   label: std::borrow::Cow<'_, str>,
                   list: Vec<&Conversation>| {
        // The user's own sections show while empty, to drop rows into.
        if list.is_empty() && !matches!(s, SidebarSection::Custom(_)) {
            return false;
        }
        let folded = s.collapsible() && collapsed.contains(&s);
        rows.push(SidebarRow::Header {
            section: s,
            label: label.into_owned().into(),
            collapsed: folded,
        });
        rows.extend(
            list.into_iter()
                .filter(|c| !folded || keep(c))
                .map(|c| row(state, index, c, selected, drafts, l)),
        );
        !folded
    };
    section(
        &mut rows,
        SidebarSection::Starred,
        t!(l, "sidebar-starred"),
        starred,
    );
    section(
        &mut rows,
        SidebarSection::Unreads,
        t!(l, "sidebar-unreads"),
        unreads,
    );
    for (sec, list) in settings.sections.iter().zip(customs) {
        section(
            &mut rows,
            SidebarSection::Custom(sec.id),
            Cow::Borrowed(sec.name.as_str()),
            list,
        );
    }
    section(
        &mut rows,
        SidebarSection::Channels,
        t!(l, "sidebar-channels"),
        channels,
    );
    let open = section(
        &mut rows,
        SidebarSection::DirectMessages,
        t!(l, "sidebar-direct-messages"),
        dms,
    );
    if open {
        if hidden_dms > 0 {
            rows.push(SidebarRow::ShowMore {
                label: t!(l, "sidebar-show-more", count = hidden_dms)
                    .into_owned()
                    .into(),
                hidden: hidden_dms,
                expanded: false,
            });
        } else if dms_expanded && total_dms > DIRECT_MESSAGES_SHOWN {
            rows.push(SidebarRow::ShowMore {
                label: t!(l, "sidebar-show-fewer").into_owned().into(),
                hidden: 0,
                expanded: true,
            });
        }
    }
    rows
}

/// The unread, non-muted conversation to open after `selected` (in row order,
/// wrapping). `None` when nothing is unread or only `selected` is.
pub fn next_unread(
    rows: &[SidebarRow],
    selected: Option<&ConversationId>,
) -> Option<ConversationId> {
    let mut order: Vec<&ConversationId> = Vec::new();
    for r in rows {
        if let SidebarRow::Conversation {
            id, unread, muted, ..
        } = r
            && *unread
            && !*muted
            && !order.contains(&id)
        {
            order.push(id);
        }
    }
    let next = match selected.and_then(|s| order.iter().position(|id| *id == s)) {
        Some(i) => order.get((i + 1) % order.len().max(1)).copied(),
        None => {
            // Not itself unread: the first unread row below it, else the first.
            let at = selected.and_then(|s| {
                rows.iter()
                    .position(|r| matches!(r, SidebarRow::Conversation { id, .. } if id == s))
            });
            let after = at.and_then(|a| {
                rows[a + 1..].iter().find_map(|r| match r {
                    SidebarRow::Conversation {
                        id, unread, muted, ..
                    } if *unread && !*muted => Some(id),
                    _ => None,
                })
            });
            after.or_else(|| order.first().copied())
        }
    };
    next.filter(|id| Some(*id) != selected).cloned()
}

pub struct Sidebar<S: SidebarSource + 'static> {
    session: Entity<S>,
    rows: Vec<SidebarRow>,
    /// The workspace on screen, shown at the top when set.
    workspace_name: Option<SharedString>,
    /// The "Scheduled" entry's count, or `None` when it is hidden.
    entry: Option<usize>,
    /// The "Reminders" entry's count, or `None` when it is hidden.
    reminders: Option<usize>,
    /// The "Threads" and "Later" entries' counts, or `None` when hidden.
    threads: Option<usize>,
    later: Option<usize>,
    /// The "Drafts" entry's count, or `None` when it is hidden.
    drafts: Option<usize>,
    /// The "People" entry is shown.
    people: bool,
    /// Each conversation row's text (prefix, name), by row.
    texts: Vec<SharedString>,
    /// The section header under the pointer: it shows its "+".
    hovered_section: Option<SidebarSection>,
    /// One focus handle per section header (a tab stop; its "+" shows while
    /// focus is within it). Custom sections get theirs while they are listed.
    section_focus: HashMap<SidebarSection, FocusHandle>,
    /// The section each row sits in, rebuilt with the rows.
    row_section: Vec<SidebarSection>,
    /// The custom sections in order, rebuilt with the rows.
    custom_order: Vec<u32>,
    /// Organise mode: rows and sections drag by their handle.
    organising: bool,
    /// The "New section" field, while organising.
    new_section: Option<Entity<crate::text_input::TextInput>>,
    /// The custom section being renamed in place, and its field.
    /// Losing focus keeps the name (the subscription commits it).
    renaming: Option<(
        u32,
        Entity<crate::text_input::TextInput>,
        gpui::Subscription,
    )>,
    /// The section menu: which custom section, where (window coordinates).
    /// `None` for the place: under the section's header (opened by keyboard).
    section_menu: Option<(u32, Option<gpui::Point<gpui::Pixels>>)>,
    /// The sidebar settings popover, where it opens (window coordinates).
    settings_at: Option<gpui::Point<gpui::Pixels>>,
    /// Keyboard focus of the open popover or menu (Escape closes it).
    overlay_focus: FocusHandle,
    /// The popover's values: show Unreads, sort (density is `density`).
    show_unreads: bool,
    sort: SidebarSort,
    /// The keyboard cursor is on this section's header.
    cursor_section: Option<SidebarSection>,
    /// The paused strip's text, while notifications are paused.
    paused: Option<String>,
    /// Row height for the user's density, refreshed when settings change.
    density: settings::Density,
    cursor: Option<ConversationId>,
    /// The cursor's row index: a conversation can sit in two sections
    /// (Starred or Unreads and its own), so the id alone is not a position.
    cursor_row: Option<usize>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    rendered: usize,
    first_rendered: usize,
    renders: usize,
    /// Set until the list has had rows, so the first fill scrolls to the top
    /// (or to the selected conversation).
    placed: bool,
}

/// The popover's options as the app-wide settings hold them: show Unreads, sort.
pub fn sidebar_options(cx: &App) -> (bool, SidebarSort) {
    cx.try_global::<SettingsStore>()
        .map_or((true, SidebarSort::Alphabetical), |s| {
            let sb = &s.current().sidebar;
            (sb.show_unreads_section, sb.sort)
        })
}

/// A conversation row's text. The prefix stays in the label's text run (one
/// element per row). The lock glyph is wider than `#`, so the channel prefix
/// carries two thin spaces and the lock only a hair space: the names start
/// within a pixel or two.
fn row_text(kind: ConversationKind, label: &str) -> SharedString {
    let prefix = match kind {
        ConversationKind::Channel => "# \u{2009}\u{2009}",
        ConversationKind::PrivateChannel => "\u{1F512} ",
        ConversationKind::Direct | ConversationKind::Group => "",
    };
    format!("{prefix}{label}").into()
}

/// The chevron a section header shows: right when folded, down when open,
/// none for a section that cannot fold (Unreads).
pub fn header_chevron(section: SidebarSection, collapsed: bool) -> Option<crate::icon::IconName> {
    section.collapsible().then_some(if collapsed {
        crate::icon::IconName::ChevRight
    } else {
        crate::icon::IconName::ChevDown
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowTone {
    Strong,
    Muted,
    Normal,
}

/// Row text tone: selected and unread rows read strong (selected wins over
/// muted, so the highlighted row keeps its contrast), muted ones recede.
fn row_tone(selected: bool, unread: bool, muted: bool) -> RowTone {
    if selected || (unread && !muted) {
        RowTone::Strong
    } else if muted {
        RowTone::Muted
    } else {
        RowTone::Normal
    }
}

impl<S: SidebarSource + 'static> Sidebar<S> {
    pub fn new(session: Entity<S>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |this, _, cx| {
            if this.refresh(cx) {
                cx.notify();
            }
        })
        .detach();
        cx.observe_global::<SettingsStore>(|this, cx| {
            if this.refresh(cx) {
                cx.notify();
            }
        })
        .detach();
        // A language switch rebuilds the section headers.
        cx.observe_global::<i18n::gpui::I18n>(|this, cx| {
            if this.refresh(cx) {
                cx.notify();
            }
        })
        .detach();
        let rows = session.read(cx).rows(cx);
        let workspace_name = session.read(cx).workspace_name(cx);
        let entry = session.read(cx).scheduled_entry();
        let reminders = session.read(cx).reminders_entry();
        let threads = session.read(cx).threads_entry();
        let later = session.read(cx).later_entry();
        let drafts = session.read(cx).drafts_entry();
        let people = session.read(cx).people_entry();
        let paused = session.read(cx).paused_label(cx);
        let options = session.read(cx).sidebar_options(cx);
        let mut this = Self {
            session,
            rows,
            workspace_name,
            entry,
            reminders,
            threads,
            later,
            drafts,
            people,
            texts: Vec::new(),
            hovered_section: None,
            section_focus: SidebarSection::ALL
                .iter()
                .map(|s| (*s, cx.focus_handle().tab_stop(true)))
                .collect(),
            row_section: Vec::new(),
            custom_order: Vec::new(),
            organising: false,
            new_section: None,
            renaming: None,
            section_menu: None,
            settings_at: None,
            overlay_focus: cx.focus_handle(),
            show_unreads: options.0,
            sort: options.1,
            cursor_section: None,
            paused,
            density: crate::density::current(cx),
            cursor: None,
            cursor_row: None,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            rendered: 0,
            first_rendered: 0,
            renders: 0,
            placed: false,
        };
        this.rebuild_texts();
        this.sync_sections(cx);
        this.place();
        this
    }

    /// How many times the sidebar has rendered (a test hook: it must not grow
    /// while the list is idle).
    pub fn renders(&self) -> usize {
        self.renders
    }

    /// Number of rows built by the virtualised list in the last frame.
    /// The row wearing the focus ring: the keyboard cursor, only while the
    /// sidebar has the keyboard focus. A mouse click never leaves a ring.
    pub fn ring_row(&self, window: &Window) -> Option<&ConversationId> {
        self.cursor
            .as_ref()
            .filter(|_| self.focus.is_focused(window))
    }

    /// The section header the keyboard cursor is on.
    pub fn cursor_section(&self) -> Option<SidebarSection> {
        self.cursor_section
    }

    /// The focus handle of a section header.
    pub fn section_focus(&self, section: SidebarSection) -> FocusHandle {
        self.section_focus
            .get(&section)
            .cloned()
            .unwrap_or_else(|| self.focus.clone())
    }

    /// Whether organise mode is on.
    pub fn organising(&self) -> bool {
        self.organising
    }

    /// Turns organise mode on (rows and sections drag by their handle, a
    /// "New section" field shows) or off (Done).
    pub fn set_organising(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_at = None;
        self.section_menu = None;
        self.organising = on;
        if on {
            let input = self
                .new_section
                .get_or_insert_with(|| {
                    let ph = t!(cx, "sidebar-new-section-placeholder").into_owned();
                    cx.new(|cx| {
                        let mut i = crate::text_input::TextInput::new(cx);
                        i.set_single_line(true);
                        i.set_placeholder(ph);
                        i
                    })
                })
                .clone();
            window.focus(&input.focus_handle(cx), cx);
            // From the top, so every section's title is in view.
            self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        } else {
            self.new_section = None;
            self.renaming = None;
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// The "New section" field, while organising.
    pub fn new_section_input(&self) -> Option<&Entity<crate::text_input::TextInput>> {
        self.new_section.as_ref()
    }

    /// The rename field and the section it renames.
    pub fn rename_input(&self) -> Option<(u32, &Entity<crate::text_input::TextInput>)> {
        self.renaming.as_ref().map(|(id, i, _)| (*id, i))
    }

    /// Opens the sidebar settings popover at `at` (window coordinates); the
    /// host offers it from the workspace menu under the sidebar's header.
    pub fn open_settings(
        &mut self,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.section_menu = None;
        self.settings_at = Some(at);
        window.focus(&self.overlay_focus, cx);
        cx.notify();
    }

    /// Whether the settings popover is open.
    pub fn settings_open(&self) -> bool {
        self.settings_at.is_some()
    }

    /// Opens the menu of custom section `id` at `at` (window coordinates),
    /// or under its header for `None`.
    pub fn open_section_menu(
        &mut self,
        id: u32,
        at: Option<gpui::Point<gpui::Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_at = None;
        self.section_menu = Some((id, at));
        window.focus(&self.overlay_focus, cx);
        cx.notify();
    }

    /// The custom section whose menu is open.
    pub fn section_menu(&self) -> Option<u32> {
        self.section_menu.map(|(id, _)| id)
    }

    fn close_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_at.take().is_some() | self.section_menu.take().is_some() {
            if self.overlay_focus.is_focused(window) {
                window.focus(&self.focus, cx);
            }
            cx.notify();
        }
    }

    fn section_op(&mut self, op: crate::SectionOp, cx: &mut Context<Self>) {
        // The source may change its rows at once (or when its file reloads).
        self.session.update(cx, |s, cx| {
            s.section_op(op, cx);
            cx.notify();
        });
    }

    fn set_option(&mut self, option: SidebarOption, cx: &mut Context<Self>) {
        // Shown at once; the settings reload confirms it.
        match option {
            SidebarOption::ShowUnreads(on) => self.show_unreads = on,
            SidebarOption::Sort(v) => self.sort = v,
            SidebarOption::Density(d) => self.density = d,
        }
        cx.notify();
        self.session
            .update(cx, |s, cx| s.set_sidebar_option(option, cx));
    }

    /// Creates a section from the "New section" field and clears it.
    pub fn submit_new_section(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.new_section.clone() else {
            return;
        };
        let name = input.read(cx).text().to_string();
        if name.trim().is_empty() {
            return;
        }
        input.update(cx, |i, cx| i.set_text("", cx));
        self.section_op(crate::SectionOp::Create { name }, cx);
    }

    /// Starts renaming custom section `id` in place.
    pub fn start_rename(&mut self, id: u32, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .rows
            .iter()
            .find_map(|r| match r {
                SidebarRow::Header {
                    section: SidebarSection::Custom(s),
                    label,
                    ..
                } if *s == id => Some(label.to_string()),
                _ => None,
            })
            .unwrap_or_default();
        let input = cx.new(|cx| {
            let mut i = crate::text_input::TextInput::new(cx);
            i.set_single_line(true);
            i.set_text(&name, cx);
            i
        });
        let handle = input.focus_handle(cx);
        window.focus(&handle, cx);
        // Clicking or tabbing away keeps the new name, as Enter does.
        let blur = cx.on_focus_out(&handle, window, |this, _, window, cx| {
            this.finish_rename(true, window, cx)
        });
        self.section_menu = None;
        self.renaming = Some((id, input, blur));
        cx.notify();
    }

    /// Ends renaming: keeps the new name, or drops it (`keep` false).
    pub fn finish_rename(&mut self, keep: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, input, blur)) = self.renaming.take() else {
            return;
        };
        drop(blur);
        if keep {
            let name = input.read(cx).text().to_string();
            self.section_op(crate::SectionOp::Rename { id, name }, cx);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// A row or header in `target` took a dragged row: into a custom section,
    /// or back to its own section from Channels or Direct messages.
    fn drop_row(&mut self, target: SidebarSection, row: &DraggedRow, cx: &mut Context<Self>) {
        let to = match target {
            SidebarSection::Custom(id) => Some(id),
            SidebarSection::Channels | SidebarSection::DirectMessages => None,
            SidebarSection::Starred | SidebarSection::Unreads => return,
        };
        self.section_op(
            crate::SectionOp::MoveConversation {
                conversation: row.conversation.0.to_string(),
                to,
            },
            cx,
        );
    }

    /// A row or header in `target` took a dragged section: it moves to the
    /// target section's place (the end below the custom sections, the start above).
    fn drop_section(
        &mut self,
        target: SidebarSection,
        sec: &DraggedSection,
        cx: &mut Context<Self>,
    ) {
        let to = match target {
            SidebarSection::Custom(id) => match self.custom_order.iter().position(|c| *c == id) {
                Some(i) => i,
                None => return,
            },
            SidebarSection::Channels | SidebarSection::DirectMessages => usize::MAX,
            SidebarSection::Starred | SidebarSection::Unreads => 0,
        };
        self.section_op(crate::SectionOp::Reorder { id: sec.id, to }, cx);
    }

    pub fn rendered_rows(&self) -> usize {
        self.rendered
    }

    /// Index of the first row built in the last frame.
    pub fn first_rendered_row(&self) -> usize {
        self.first_rendered
    }

    /// Rebuilds the rows; false when they came out identical, so an unrelated
    /// session change does not repaint the list.
    fn refresh(&mut self, cx: &mut App) -> bool {
        // Read here, never in `render`: a read while rendering would repaint the
        // sidebar on every session notification.
        let workspace_name = self.session.read(cx).workspace_name(cx);
        let entry = self.session.read(cx).scheduled_entry();
        let reminders = self.session.read(cx).reminders_entry();
        let threads = self.session.read(cx).threads_entry();
        let later = self.session.read(cx).later_entry();
        let drafts = self.session.read(cx).drafts_entry();
        let people = self.session.read(cx).people_entry();
        let rows = self.session.read(cx).rows(cx);
        let paused = self.session.read(cx).paused_label(cx);
        let density = crate::density::current(cx);
        let (show_unreads, sort) = self.session.read(cx).sidebar_options(cx);
        if rows == self.rows
            && density == self.density
            && show_unreads == self.show_unreads
            && sort == self.sort
            && workspace_name == self.workspace_name
            && entry == self.entry
            && reminders == self.reminders
            && threads == self.threads
            && later == self.later
            && drafts == self.drafts
            && people == self.people
            && paused == self.paused
        {
            return false;
        }
        self.density = density;
        self.show_unreads = show_unreads;
        self.sort = sort;
        self.workspace_name = workspace_name;
        self.paused = paused;
        self.reminders = reminders;
        self.threads = threads;
        self.later = later;
        self.drafts = drafts;
        self.people = people;
        self.entry = entry;
        self.rows = rows;
        self.rebuild_texts();
        self.sync_sections(cx);
        self.place();
        true
    }

    /// Keeps one focus handle per listed custom section, and the order.
    fn sync_sections(&mut self, cx: &mut App) {
        self.custom_order.clear();
        for r in &self.rows {
            if let SidebarRow::Header {
                section: SidebarSection::Custom(id),
                ..
            } = r
            {
                self.custom_order.push(*id);
            }
        }
        let order = &self.custom_order;
        self.section_focus.retain(|s, _| match s {
            SidebarSection::Custom(id) => order.contains(id),
            _ => true,
        });
        for id in order {
            self.section_focus
                .entry(SidebarSection::Custom(*id))
                .or_insert_with(|| cx.focus_handle().tab_stop(true));
        }
        if self
            .renaming
            .as_ref()
            .is_some_and(|(id, _, _)| !order.contains(id))
        {
            self.renaming = None;
        }
        if self
            .section_menu
            .is_some_and(|(id, _)| !order.contains(&id))
        {
            self.section_menu = None;
        }
    }

    /// Rebuilds the row texts: built once per rows change, not per frame.
    fn rebuild_texts(&mut self) {
        let mut current = SidebarSection::Channels;
        self.row_section = self
            .rows
            .iter()
            .map(|r| {
                if let SidebarRow::Header { section, .. } = r {
                    current = *section;
                }
                current
            })
            .collect();
        self.texts = self
            .rows
            .iter()
            .map(|r| match r {
                SidebarRow::Conversation { kind, label, .. } => row_text(*kind, label),
                _ => SharedString::default(),
            })
            .collect();
    }

    fn place(&mut self) {
        if !self.placed && !self.rows.is_empty() {
            // The first fill opens at the top, or at the selected conversation.
            self.placed = true;
            let at = self
                .rows
                .iter()
                .position(|r| matches!(r, SidebarRow::Conversation { selected: true, .. }))
                .unwrap_or(0);
            self.scroll
                .scroll_to_item_strict(at, gpui::ScrollStrategy::Top);
        }
    }

    fn id_at(&self, ix: usize) -> Option<ConversationId> {
        match self.rows.get(ix)? {
            SidebarRow::Conversation { id, .. } => Some(id.clone()),
            SidebarRow::Header { .. } | SidebarRow::ShowMore { .. } => None,
        }
    }

    fn cursor_ix(&self) -> Option<usize> {
        if let Some(sec) = self.cursor_section {
            return self
                .rows
                .iter()
                .position(|r| matches!(r, SidebarRow::Header { section, .. } if *section == sec));
        }
        let cur = self.cursor.as_ref()?;
        if let Some(i) = self.cursor_row
            && matches!(self.rows.get(i), Some(SidebarRow::Conversation { id, .. }) if id == cur)
        {
            return Some(i);
        }
        self.rows
            .iter()
            .position(|r| matches!(r, SidebarRow::Conversation { id, .. } if id == cur))
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let n = self.rows.len();
        // From the selected row when the mouse (not the keyboard) chose it.
        let mut ix = self.cursor_ix().or_else(|| {
            self.rows
                .iter()
                .position(|r| matches!(r, SidebarRow::Conversation { selected: true, .. }))
        });
        loop {
            ix = match (ix, forward) {
                (None, true) => Some(0),
                (None, false) => n.checked_sub(1),
                (Some(i), true) => Some(i + 1),
                (Some(i), false) => i.checked_sub(1),
            };
            match ix {
                Some(i) if i < n => {
                    if let Some(id) = self.id_at(i) {
                        self.cursor = Some(id);
                        self.cursor_section = None;
                        self.cursor_row = Some(i);
                        self.scroll.scroll_to_item(i, gpui::ScrollStrategy::Nearest);
                        break;
                    }
                    if let Some(SidebarRow::Header { section, .. }) = self.rows.get(i) {
                        self.cursor = None;
                        self.cursor_section = Some(*section);
                        self.cursor_row = Some(i);
                        self.scroll.scroll_to_item(i, gpui::ScrollStrategy::Nearest);
                        break;
                    }
                }
                _ => break,
            }
        }
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    fn next_unread(&mut self, _: &NextUnread, _: &mut Window, cx: &mut Context<Self>) {
        let selected = self.rows.iter().find_map(|r| match r {
            SidebarRow::Conversation { id, selected, .. } if *selected => Some(id.clone()),
            _ => None,
        });
        if let Some(id) = next_unread(&self.rows, selected.as_ref()) {
            self.cursor = Some(id.clone());
            self.cursor_section = None;
            self.cursor_row = None;
            self.session.update(cx, |s, cx| s.open(id, cx));
        }
    }

    fn open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        // Enter on a focused header folds it; on its focused "+" it asks for
        // the picker (the binding wins over the elements' own key handlers).
        let heads: Vec<(SidebarSection, FocusHandle)> = self
            .section_focus
            .iter()
            .map(|(s, f)| (*s, f.clone()))
            .collect();
        for (section, focus) in heads {
            if focus.is_focused(window) {
                if section.collapsible() {
                    self.session
                        .update(cx, |s, cx| s.toggle_section(section, cx));
                }
                return;
            }
            if focus.contains_focused(window, cx) {
                match section {
                    SidebarSection::Channels => cx.emit(SidebarEvent::BrowseChannels),
                    SidebarSection::DirectMessages => cx.emit(SidebarEvent::BrowsePeople),
                    // Its "…": the section menu, under the header.
                    SidebarSection::Custom(id) => self.open_section_menu(id, None, window, cx),
                    _ => {}
                }
                return;
            }
        }
        if let Some(section) = self.cursor_section {
            if section.collapsible() {
                self.session
                    .update(cx, |s, cx| s.toggle_section(section, cx));
            }
            return;
        }
        if let Some(id) = self.cursor.clone() {
            self.session.update(cx, |s, cx| s.open(id, cx));
        }
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let ring_ix = self
            .focus
            .is_focused(window)
            .then(|| self.cursor_ix())
            .flatten();
        self.rendered = range.len();
        self.first_rendered = range.start;
        let theme = cx.theme().clone();
        let tokens = parla_tokens(&theme);
        let status = theme.status();
        // One height for the whole batch; the uniform list measures the first row.
        let row_h = crate::density::row_height(self.density);
        let organising = self.organising;
        // Where a dragged row or section would land.
        let drop_fill = tokens.signal_soft;
        range
            .filter_map(|ix| self.rows.get(ix).map(|r| (ix, r)))
            .map(|(ix, r)| match r {
                SidebarRow::Header {
                    section,
                    label,
                    collapsed,
                } => {
                    let section = *section;
                    let foldable = section.collapsible();
                    let focus = self.section_focus.get(&section).unwrap_or(&self.focus);
                    let custom = match section {
                        SidebarSection::Custom(id) => Some(id),
                        _ => None,
                    };
                    let rename = self
                        .renaming
                        .as_ref()
                        .filter(|(id, _, _)| Some(*id) == custom)
                        .map(|(_, input, _)| input.clone());
                    let keyboard = ring_ix == Some(ix);
                    let hovered = self.hovered_section == Some(section);
                    // The "+" shows on hover and while the keyboard is on the
                    // header or inside it (one icon less per header in every
                    // idle frame).
                    let show_add = hovered || keyboard || focus.contains_focused(window, cx);
                    // Channels and Direct messages carry a "+" at the end.
                    let add = match section {
                        SidebarSection::Channels => {
                            Some(("sidebar-add-channels", true, SidebarEvent::BrowseChannels))
                        }
                        SidebarSection::DirectMessages => Some((
                            "sidebar-add-direct_messages",
                            false,
                            SidebarEvent::BrowsePeople,
                        )),
                        _ => None,
                    }
                    .filter(|_| show_add);
                    div()
                        .id(("sidebar-header", ix))
                        .debug_selector(move || section.selector().to_string())
                        .track_focus(focus)
                        .focus_visible(crate::controls::focus_style(theme.colors()))
                        .when(keyboard, |d| {
                            d.shadow(crate::controls::ring(theme.colors()))
                        })
                        .w_full()
                        .h(row_h)
                        // A header without a chevron keeps its room, so every
                        // label starts at the same x.
                        // A custom header's handle lines up with the rows' handles.
                        .when(foldable && !(organising && custom.is_some()), |d| {
                            d.pl(themes::metrics::SPACE_3)
                        })
                        .when(organising && custom.is_some(), |d| {
                            d.pl(themes::metrics::SPACE_5)
                        })
                        .when(!foldable, |d| d.pl(themes::metrics::SPACE_8))
                        .relative()
                        .pr(themes::metrics::SPACE_3)
                        .flex()
                        .items_center()
                        .gap(themes::metrics::SPACE_2)
                        .rounded(themes::metrics::RADIUS_SM)
                        .when(foldable, |d| d.cursor_pointer())
                        .when(hovered, |d| d.bg(tokens.sidebar_hover))
                        .text_size(themes::metrics::FONT_SIZE_XS)
                        .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                        .text_color(tokens.sidebar_muted)
                        .when(organising && custom.is_some(), |d| {
                            d.child(
                                crate::icon::Icon::new(crate::icon::IconName::MoreV)
                                    .size(themes::metrics::ICON_SIZE_SM)
                                    .color(tokens.sidebar_muted),
                            )
                        })
                        .when_some(header_chevron(section, *collapsed), |d, icon| {
                            d.child(
                                crate::icon::Icon::new(icon)
                                    .size(themes::metrics::ICON_SIZE_SM)
                                    .color(tokens.sidebar_muted),
                            )
                        })
                        .when(rename.is_none(), |d| d.child(label.clone()))
                        .when_some(rename, |d, input| {
                            d.child(
                                div()
                                    .key_context(FIELD_KEY_CONTEXT)
                                    .debug_selector(|| "sidebar-rename".to_string())
                                    .flex_1()
                                    .min_w_0()
                                    .px(themes::metrics::SPACE_2)
                                    .rounded(themes::metrics::RADIUS_SM)
                                    .border_1()
                                    .border_color(tokens.signal)
                                    .text_color(tokens.sidebar_text_strong)
                                    .on_key_down(cx.listener(
                                        |this, ev: &gpui::KeyDownEvent, window, cx| {
                                            match ev.keystroke.key.as_str() {
                                                "enter" => this.finish_rename(true, window, cx),
                                                "escape" => this.finish_rename(false, window, cx),
                                                _ => return,
                                            }
                                            cx.stop_propagation();
                                        },
                                    ))
                                    .child(input),
                            )
                        })
                        .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                            let next = if *over {
                                Some(section)
                            } else if this.hovered_section == Some(section) {
                                None
                            } else {
                                this.hovered_section
                            };
                            if next != this.hovered_section {
                                this.hovered_section = next;
                                cx.notify();
                            }
                        }))
                        .when_some(add, |d, (id, channels, ev)| {
                            d.child(
                                div()
                                    .ml_auto()
                                    .id(id)
                                    .debug_selector(move || id.to_string())
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .size(themes::metrics::LAYOUT_ICON_BUTTON_SM)
                                    .rounded(themes::metrics::RADIUS_SM)
                                    .tab_stop(true)
                                    .focusable()
                                    .focus_visible(crate::controls::focus_style(theme.colors()))
                                    .hover(|h| h.bg(tokens.sidebar_active))
                                    .tooltip(move |w, cx| {
                                        let text = if channels {
                                            t!(cx, "sidebar-find-channel")
                                        } else {
                                            t!(cx, "sidebar-message-someone")
                                        }
                                        .into_owned();
                                        crate::reactions::tooltip_view(text)(w, cx)
                                    })
                                    .child(
                                        crate::icon::Icon::new(crate::icon::IconName::Plus)
                                            .size(themes::metrics::ICON_SIZE_SM)
                                            .color(tokens.sidebar_muted),
                                    )
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.emit(ev.clone());
                                    })),
                            )
                        })
                        // Enter and Space reach the header through the
                        // sidebar's `Open` binding.
                        .when(foldable, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.session
                                    .update(cx, |s, cx| s.toggle_section(section, cx));
                            }))
                        })
                        // A custom section's menu: right click, or its "..." while organising.
                        .when_some(custom, |d, id| {
                            d.on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                    this.open_section_menu(id, Some(ev.position), window, cx);
                                }),
                            )
                        })
                        // The "…" shows while organising, and like the "+" on
                        // hover or while the keyboard is on the header.
                        .when_some(custom.filter(|_| organising || show_add), |d, id| {
                            d.child(
                                div()
                                    .ml_auto()
                                    .id(("sidebar-section-more", ix))
                                    .debug_selector(move || format!("sidebar-section-more-{id}"))
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .size(themes::metrics::LAYOUT_ICON_BUTTON_SM)
                                    .rounded(themes::metrics::RADIUS_SM)
                                    .tab_stop(true)
                                    .focusable()
                                    .focus_visible(crate::controls::focus_style(theme.colors()))
                                    .hover(|h| h.bg(tokens.sidebar_active))
                                    .child(
                                        crate::icon::Icon::new(crate::icon::IconName::More)
                                            .size(themes::metrics::ICON_SIZE_SM)
                                            .color(tokens.sidebar_muted),
                                    )
                                    .on_click(cx.listener(
                                        move |this, ev: &gpui::ClickEvent, window, cx| {
                                            cx.stop_propagation();
                                            this.open_section_menu(
                                                id,
                                                Some(ev.position()),
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                            )
                        })
                        // Opened from the keyboard: the menu hangs under the header.
                        .when_some(
                            custom.filter(|id| self.section_menu == Some((*id, None))),
                            |d, _| d.children(self.section_menu_view(cx)),
                        )
                        .when_some(custom.filter(|_| organising), |d, id| {
                            d.on_drag(
                                DraggedSection {
                                    id,
                                    label: label.clone(),
                                },
                                |sec: &DraggedSection, _, _, cx| {
                                    let label = sec.label.clone();
                                    cx.new(|_| DragPreview(label))
                                },
                            )
                        })
                        .when(organising, |d| {
                            d.drag_over::<DraggedRow>(move |st, _, _, _| st.bg(drop_fill))
                                .drag_over::<DraggedSection>(move |st, _, _, _| st.bg(drop_fill))
                                .on_drop(cx.listener(move |this, row: &DraggedRow, _, cx| {
                                    this.drop_row(section, row, cx)
                                }))
                                .on_drop(cx.listener(move |this, sec: &DraggedSection, _, cx| {
                                    this.drop_section(section, sec, cx)
                                }))
                        })
                        .into_any_element()
                }
                SidebarRow::ShowMore { label, .. } => div()
                    .id(("sidebar-show-more", ix))
                    .debug_selector(|| "sidebar-show-more".to_string())
                    .w_full()
                    .h(row_h)
                    .px(themes::metrics::SPACE_5)
                    .flex()
                    .items_center()
                    .text_size(themes::metrics::FONT_SIZE_XS)
                    .text_color(tokens.sidebar_muted)
                    .cursor_pointer()
                    .child(label.clone())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session
                            .update(cx, |s, cx| s.toggle_direct_messages(cx));
                    }))
                    .into_any_element(),
                SidebarRow::Conversation {
                    id,
                    label,
                    kind,
                    unread,
                    mentions,
                    selected,
                    muted,
                    draft,
                    level,
                    presence,
                    dot_slot,
                    huddle,
                } => {
                    let cursor = ring_ix == Some(ix);
                    let in_section = self
                        .row_section
                        .get(ix)
                        .copied()
                        .unwrap_or(SidebarSection::Channels);
                    // Selected and unread rows read strong; muted ones recede.
                    let strong = *selected || (*unread && !*muted);
                    let fg = match row_tone(*selected, *unread, *muted) {
                        RowTone::Strong => tokens.sidebar_text_strong,
                        RowTone::Muted => tokens.sidebar_muted,
                        RowTone::Normal => tokens.sidebar_text,
                    };
                    // One element per row (the inset comes from the margin).
                    let mut el = div()
                        .id(("sidebar-row", ix))
                        .debug_selector(|| format!("sidebar-row-{}", id.0))
                        .h(row_h)
                        .w_full()
                        .cursor_pointer()
                        .px(themes::metrics::SPACE_5)
                        .flex()
                        .items_center()
                        .gap(themes::metrics::SPACE_4)
                        .rounded(themes::metrics::RADIUS_SM)
                        .text_size(themes::metrics::FONT_SIZE_MD)
                        .text_color(fg)
                        .when(*selected || cursor, |d| d.bg(tokens.sidebar_active))
                        // Keyboard cursor: the focus ring, inside the row.
                        .when(cursor, |d| d.shadow(crate::controls::ring(theme.colors())))
                        .when(!*selected && !cursor, |d| {
                            d.hover(|h| h.bg(tokens.sidebar_hover))
                        })
                        .when(strong, |d| {
                            d.font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                        })
                        // Organise mode: the handle, and the row drags.
                        .when(organising, |d| {
                            d.child(
                                crate::icon::Icon::new(crate::icon::IconName::MoreV)
                                    .size(themes::metrics::ICON_SIZE_SM)
                                    .color(tokens.sidebar_muted),
                            )
                            .on_drag(
                                DraggedRow {
                                    conversation: id.clone(),
                                    label: label.clone(),
                                },
                                |row: &DraggedRow, _, _, cx| {
                                    let label = row.label.clone();
                                    cx.new(|_| DragPreview(label))
                                },
                            )
                            .drag_over::<DraggedRow>(move |st, _, _, _| st.bg(drop_fill))
                            .drag_over::<DraggedSection>(move |st, _, _, _| st.bg(drop_fill))
                            .on_drop(cx.listener(move |this, row: &DraggedRow, _, cx| {
                                this.drop_row(in_section, row, cx)
                            }))
                            .on_drop(cx.listener(
                                move |this, sec: &DraggedSection, _, cx| {
                                    this.drop_section(in_section, sec, cx)
                                },
                            ))
                        })
                        .when_some(*presence, |d, active| {
                            d.child(presence_dot(
                                ("sidebar-presence", ix),
                                active,
                                tokens.presence_active,
                                tokens.presence_away,
                            ))
                        })
                        // Keep names aligned while a person's presence is unknown.
                        .when(presence.is_none() && *dot_slot, |d| {
                            d.child(div().flex_none().size(px(8.0)))
                        })
                        .child(
                            // One line, cut with an ellipsis at the column edge.
                            div()
                                .id(("sidebar-label", ix))
                                .debug_selector(|| format!("sidebar-label-{ix}"))
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(
                                    self.texts
                                        .get(ix)
                                        .cloned()
                                        .unwrap_or_else(|| row_text(*kind, label)),
                                ),
                        );
                    if *draft {
                        el = el.child(
                            div()
                                .debug_selector(|| format!("sidebar-draft-icon-{ix}"))
                                .flex_none()
                                .child(
                                    crate::icon::Icon::new(crate::icon::IconName::Pencil)
                                        .size(themes::metrics::ICON_SIZE_SM)
                                        .color(tokens.sidebar_muted),
                                ),
                        );
                    }
                    if let Some(mark) = level_mark(*level) {
                        el = el.child(
                            div()
                                .debug_selector(|| format!("sidebar-level-{ix}"))
                                .px_1()
                                .text_xs()
                                .opacity(0.75)
                                .child(mark),
                        );
                    }
                    if let Some(n) = *huddle {
                        el = el.child(
                            div()
                                .debug_selector(|| format!("sidebar-huddle-{ix}"))
                                .flex()
                                .items_center()
                                .gap_1()
                                .px_1()
                                .text_xs()
                                .child(
                                    div()
                                        .size(px(8.0))
                                        .rounded_full()
                                        .bg(tokens.presence_active),
                                )
                                .child(n.to_string()),
                        );
                    }
                    if *mentions > 0 {
                        el = el.child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .min_w(themes::metrics::SPACE_7)
                                .h(themes::metrics::SPACE_7)
                                .px(themes::metrics::SPACE_3)
                                .rounded_full()
                                .bg(status.error)
                                .text_color(tokens.danger_text)
                                .text_size(themes::metrics::FONT_SIZE_XS)
                                .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                                .child(mentions.to_string()),
                        );
                    }
                    let (id, menu_id) = (id.clone(), id.clone());
                    el.on_click(cx.listener(move |this, _, _, cx| {
                        // A click selects; only the keyboard moves the ring.
                        this.cursor = None;
                        this.cursor_section = None;
                        this.session.update(cx, |s, cx| s.open(id.clone(), cx));
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |_, ev: &MouseDownEvent, _, cx| {
                            cx.emit(SidebarEvent::ContextMenu {
                                conversation: menu_id.clone(),
                                position: ev.position,
                            });
                        }),
                    )
                    .into_any_element()
                }
            })
            .collect()
    }
}

impl<S: SidebarSource + 'static> gpui::EventEmitter<SidebarEvent> for Sidebar<S> {}

impl<S: SidebarSource + 'static> Focusable for Sidebar<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: SidebarSource + 'static> Render for Sidebar<S> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let tokens = parla_tokens(cx.theme());
        let row_h = crate::density::row_height(self.density);
        let colors = *cx.theme().colors();
        let workspace_name = self.workspace_name.clone().map(|name| {
            div()
                .w_full()
                .flex_none()
                .h(themes::metrics::LAYOUT_SIDEBAR_HEADER)
                .pl(themes::metrics::SPACE_7)
                .pr(themes::metrics::SPACE_5)
                .flex()
                .items_center()
                .gap(themes::metrics::SPACE_4)
                .border_b_1()
                .border_color(tokens.sidebar_border)
                .child(
                    // The name opens the workspace menu.
                    div()
                        .id("sidebar-workspace-name")
                        .debug_selector(|| "sidebar-workspace-name".to_string())
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(themes::metrics::SPACE_3)
                        .cursor_pointer()
                        .tab_stop(true)
                        .focusable()
                        .focus_visible(crate::controls::focus_style(&colors))
                        .text_size(themes::metrics::FONT_SIZE_XL)
                        .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                        .text_color(tokens.sidebar_text_strong)
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(name),
                        )
                        .child(
                            crate::icon::Icon::new(crate::icon::IconName::ChevDown)
                                .size(themes::metrics::ICON_SIZE_MD)
                                .color(tokens.sidebar_muted),
                        )
                        .on_click(cx.listener(|_, ev: &gpui::ClickEvent, _, cx| {
                            cx.emit(SidebarEvent::WorkspaceMenu {
                                position: ev.position(),
                            });
                        })),
                )
                .when(self.organising, |d| {
                    // On the sidebar ground like New message (an accent fill
                    // would sink into the navy sidebar).
                    d.child(
                        div()
                            .id("sidebar-done")
                            .debug_selector(|| "sidebar-done".to_string())
                            .flex()
                            .flex_none()
                            .items_center()
                            .h(themes::metrics::LAYOUT_TITLEBAR_BUTTON)
                            .px(themes::metrics::SPACE_5)
                            .rounded(themes::metrics::RADIUS_MD)
                            .cursor_pointer()
                            .tab_stop(true)
                            .focusable()
                            .focus_visible(crate::controls::focus_style(&colors))
                            .role(gpui::Role::Button)
                            .bg(tokens.sidebar_hover)
                            .hover(|h| h.bg(tokens.sidebar_active))
                            .text_size(themes::metrics::FONT_SIZE_SM)
                            .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                            .text_color(tokens.sidebar_text_strong)
                            .child(t!(cx, "sidebar-done").into_owned())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_organising(false, window, cx)
                            })),
                    )
                })
                .when(!self.organising, |d| {
                    d.child(
                        div()
                            .id("sidebar-new-message")
                            .debug_selector(|| "sidebar-new-message".to_string())
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(themes::metrics::LAYOUT_TITLEBAR_BUTTON)
                            .rounded(themes::metrics::RADIUS_MD)
                            .cursor_pointer()
                            .tab_stop(true)
                            .focusable()
                            .focus_visible(crate::controls::focus_style(&colors))
                            .bg(tokens.sidebar_hover)
                            .hover(|h| h.bg(tokens.sidebar_active))
                            .tooltip(|w, cx| {
                                crate::reactions::tooltip_view(
                                    t!(cx, "sidebar-new-message").into_owned(),
                                )(w, cx)
                            })
                            .child(
                                crate::icon::Icon::new(crate::icon::IconName::Compose)
                                    .size(themes::metrics::ICON_SIZE_LG)
                                    .color(tokens.sidebar_text_strong),
                            )
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::NewMessage))),
                    )
                })
        });
        let jump = t!(cx, "sidebar-jump").into_owned();
        let search = div()
            .w_full()
            .flex_none()
            .pt(themes::metrics::SPACE_5)
            .pb(themes::metrics::SPACE_2)
            .child(
                div()
                    .id("sidebar-search")
                    .debug_selector(|| "sidebar-search".to_string())
                    .mx(themes::metrics::SPACE_5)
                    .h(themes::metrics::LAYOUT_SIDEBAR_SEARCH)
                    .px(themes::metrics::SPACE_4)
                    .flex()
                    .items_center()
                    .rounded(themes::metrics::RADIUS_MD)
                    .border_1()
                    .border_color(tokens.sidebar_border)
                    .bg(tokens.sidebar_hover)
                    .text_size(themes::metrics::FONT_SIZE_SM)
                    .text_color(tokens.sidebar_muted)
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::QuickSwitch)))
                    .child(jump),
            );
        // A quick-link row (Scheduled, Reminders, Threads, Later): same shell as
        // a conversation row.
        let entry_row = |id: &'static str, label: String| {
            div()
                .id(id)
                .debug_selector(|| id.to_string())
                .w_full()
                .flex_none()
                .h(row_h)
                .px(themes::metrics::SPACE_4)
                .cursor_pointer()
                .child(
                    div()
                        .size_full()
                        .hover(|h| h.bg(tokens.sidebar_hover))
                        .px(themes::metrics::SPACE_5)
                        .flex()
                        .items_center()
                        .rounded(themes::metrics::RADIUS_SM)
                        .text_size(themes::metrics::FONT_SIZE_MD)
                        .text_color(tokens.sidebar_text)
                        .child(label),
                )
        };
        let entry = self.entry.map(|n| {
            let label = if n == 0 {
                t!(cx, "sidebar-scheduled").into_owned()
            } else {
                t!(cx, "sidebar-scheduled-count", count = n).into_owned()
            };
            entry_row("sidebar-scheduled", label).on_click(cx.listener(|this, _, _, cx| {
                this.session.update(cx, |s, cx| s.open_scheduled(cx));
            }))
        });
        let reminders = self.reminders.map(|n| {
            let label = if n == 0 {
                t!(cx, "sidebar-reminders").into_owned()
            } else {
                t!(cx, "sidebar-reminders-count", count = n).into_owned()
            };
            entry_row("sidebar-reminders", label).on_click(cx.listener(|this, _, _, cx| {
                this.session.update(cx, |s, cx| s.open_reminders(cx));
            }))
        });
        let threads = self.threads.map(|n| {
            let label = if n == 0 {
                t!(cx, "sidebar-threads").into_owned()
            } else {
                t!(cx, "sidebar-threads-count", count = n).into_owned()
            };
            entry_row("sidebar-threads", label).on_click(cx.listener(|this, _, _, cx| {
                this.session.update(cx, |s, cx| s.open_threads(cx));
            }))
        });
        let later = self.later.map(|n| {
            let label = if n == 0 {
                t!(cx, "sidebar-later").into_owned()
            } else {
                t!(cx, "sidebar-later-count", count = n).into_owned()
            };
            entry_row("sidebar-later", label).on_click(cx.listener(|this, _, _, cx| {
                this.session.update(cx, |s, cx| s.open_later(cx));
            }))
        });
        // As on the board: a pencil, "Drafts" and the count at the end.
        let drafts = self.drafts.map(|n| {
            div()
                .id("sidebar-drafts")
                .debug_selector(|| "sidebar-drafts".to_string())
                .w_full()
                .flex_none()
                .h(row_h)
                .px(themes::metrics::SPACE_4)
                .cursor_pointer()
                .child(
                    div()
                        .size_full()
                        .hover(|h| h.bg(tokens.sidebar_hover))
                        .px(themes::metrics::SPACE_5)
                        .flex()
                        .items_center()
                        .gap(themes::metrics::SPACE_4)
                        .rounded(themes::metrics::RADIUS_SM)
                        .text_size(themes::metrics::FONT_SIZE_MD)
                        .text_color(tokens.sidebar_text)
                        .child(
                            div()
                                .debug_selector(|| "sidebar-drafts-icon".to_string())
                                .child(
                                    crate::icon::Icon::new(crate::icon::IconName::Pencil)
                                        .size(themes::metrics::ICON_SIZE_MD)
                                        .color(tokens.sidebar_muted),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(t!(cx, "sidebar-drafts").into_owned()),
                        )
                        .child(
                            div()
                                .debug_selector(|| "sidebar-drafts-count".to_string())
                                .text_size(themes::metrics::FONT_SIZE_XS)
                                .font_weight(themes::metrics::FONT_WEIGHT_MEDIUM)
                                .text_color(tokens.sidebar_muted)
                                .child(n.to_string()),
                        ),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.session.update(cx, |s, cx| s.open_drafts(cx));
                }))
        });
        // "People" with its icon, a tab stop that Enter and Space open.
        let people = self.people.then(|| {
            crate::controls::on_activate(
                div()
                    .id("sidebar-people")
                    .debug_selector(|| "sidebar-people".to_string())
                    .w_full()
                    .flex_none()
                    .h(row_h)
                    .px(themes::metrics::SPACE_4)
                    .cursor_pointer()
                    .tab_stop(true)
                    .focusable()
                    .rounded(themes::metrics::RADIUS_SM)
                    .focus_visible(crate::controls::focus_style(&colors))
                    .child(
                        div()
                            .size_full()
                            .hover(|h| h.bg(tokens.sidebar_hover))
                            .px(themes::metrics::SPACE_5)
                            .flex()
                            .items_center()
                            .gap(themes::metrics::SPACE_4)
                            .rounded(themes::metrics::RADIUS_SM)
                            .text_size(themes::metrics::FONT_SIZE_MD)
                            .text_color(tokens.sidebar_text)
                            .child(
                                crate::icon::Icon::new(crate::icon::IconName::Users)
                                    .size(themes::metrics::ICON_SIZE_MD)
                                    .color(tokens.sidebar_muted),
                            )
                            .child(t!(cx, "sidebar-people").into_owned()),
                    ),
                cx,
                |this, _, cx| this.session.update(cx, |s, cx| s.open_people(cx)),
            )
        });
        let paused = self.paused.clone().map(|label| {
            div()
                .id("sidebar-paused")
                .debug_selector(|| "sidebar-paused".to_string())
                .w_full()
                .px(themes::metrics::SPACE_7)
                .py_1()
                .flex()
                .flex_col()
                .gap_1()
                .text_xs()
                .text_color(tokens.sidebar_muted)
                .child(div().w_full().child(label))
                .child(
                    div()
                        .id("sidebar-resume")
                        .debug_selector(|| "sidebar-resume".to_string())
                        .flex_none()
                        .cursor_pointer()
                        .text_color(tokens.sidebar_text_strong)
                        .child(t!(cx, "notify-resume").into_owned())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |s, cx| s.resume_notifications(cx));
                        })),
                )
        });
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::next_unread))
            .on_action(cx.listener(Self::open))
            .size_full()
            .bg(tokens.sidebar_background)
            .flex()
            .flex_col()
            .children(workspace_name)
            .child(search)
            .children(paused)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .flex_none()
                    .pt(themes::metrics::SPACE_3)
                    .children(entry)
                    .children(reminders)
                    .children(threads)
                    .children(later)
                    .children(drafts)
                    .children(people),
            )
            .child(
                uniform_list(
                    "sidebar-list",
                    self.rows.len(),
                    cx.processor(Self::render_rows),
                )
                .track_scroll(&self.scroll)
                .w_full()
                .px(themes::metrics::SPACE_4)
                .flex_1()
                .min_h_0(),
            )
            .children(self.new_section_field(cx))
            .children(self.settings_popover(cx))
            // A menu opened from the keyboard hangs under its header instead.
            .children(
                self.section_menu
                    .and_then(|(_, at)| at)
                    .and_then(|_| self.section_menu_view(cx)),
            )
    }
}

/// What a section menu entry does.
type MenuAction<S> = Box<dyn Fn(&mut Sidebar<S>, &mut Window, &mut Context<Sidebar<S>>)>;

impl<S: SidebarSource + 'static> Sidebar<S> {
    /// Organise mode's "New section" field under the list.
    fn new_section_field(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<S>> {
        let input = self.new_section.clone()?;
        let tokens = parla_tokens(cx.theme());
        // The theme's muted text is too faint on the sidebar ground.
        input.update(cx, |i, _| i.set_placeholder_color(tokens.sidebar_text));
        Some(
            div()
                .flex_none()
                .w_full()
                .px(themes::metrics::SPACE_5)
                .pt(themes::metrics::SPACE_3)
                .pb(themes::metrics::SPACE_5)
                .flex()
                .flex_col()
                .gap(themes::metrics::SPACE_2)
                .child(
                    div()
                        .text_size(themes::metrics::FONT_SIZE_XS)
                        .text_color(tokens.sidebar_muted)
                        .child(t!(cx, "sidebar-organise-hint").into_owned()),
                )
                .child(
                    div()
                        .id("sidebar-new-section")
                        .debug_selector(|| "sidebar-new-section".to_string())
                        .key_context(FIELD_KEY_CONTEXT)
                        .h(themes::metrics::LAYOUT_SIDEBAR_SEARCH)
                        .px(themes::metrics::SPACE_4)
                        .flex()
                        .items_center()
                        .gap(themes::metrics::SPACE_3)
                        .rounded(themes::metrics::RADIUS_MD)
                        .border_1()
                        .border_color(tokens.signal)
                        .bg(tokens.sidebar_hover)
                        .text_size(themes::metrics::FONT_SIZE_SM)
                        .text_color(tokens.sidebar_text_strong)
                        .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, window, cx| {
                            match ev.keystroke.key.as_str() {
                                "enter" => this.submit_new_section(cx),
                                // Clears the name first; leaves organising
                                // only from an empty field.
                                "escape" => {
                                    let input = this.new_section.clone();
                                    match input.filter(|i| !i.read(cx).text().is_empty()) {
                                        Some(i) => i.update(cx, |i, cx| i.set_text("", cx)),
                                        None => this.set_organising(false, window, cx),
                                    }
                                }
                                _ => return,
                            }
                            cx.stop_propagation();
                        }))
                        .child(
                            crate::icon::Icon::new(crate::icon::IconName::Plus)
                                .size(themes::metrics::ICON_SIZE_SM)
                                .color(tokens.sidebar_muted),
                        )
                        .child(div().flex_1().min_w_0().child(input))
                        .child(
                            div()
                                .text_size(themes::metrics::FONT_SIZE_XS)
                                .text_color(tokens.sidebar_muted)
                                .child(t!(cx, "sidebar-new-section-hint").into_owned()),
                        ),
                ),
        )
    }

    /// The raised card a popover or menu sits in, closed by a click outside
    /// or Escape.
    /// At `at` (window coordinates), or for `None` under the right end of
    /// the element it is a child of. Its own key context keeps the list's
    /// Up, Down, Enter and Space bindings off its controls.
    fn overlay(
        &self,
        id: &'static str,
        at: Option<gpui::Point<gpui::Pixels>>,
        role: gpui::Role,
        label: String,
        body: Vec<gpui::AnyElement>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = *cx.theme().colors();
        let anchored = match at {
            Some(at) => gpui::anchored().position(at),
            None => gpui::anchored().anchor(gpui::Anchor::TopRight),
        };
        let card = gpui::deferred(
            anchored
                .snap_to_window_with_margin(themes::metrics::SPACE_3)
                .child(
                    div()
                        .id(id)
                        .debug_selector(move || id.to_string())
                        .key_context(FIELD_KEY_CONTEXT)
                        .track_focus(&self.overlay_focus)
                        .role(role)
                        .aria_label(label)
                        .occlude()
                        .min_w(themes::metrics::LAYOUT_POPOVER_SM)
                        .p(themes::metrics::SPACE_3)
                        .flex()
                        .flex_col()
                        .rounded(themes::metrics::RADIUS_LG)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.elevated_surface_background)
                        .text_color(colors.text)
                        .text_size(themes::metrics::FONT_SIZE_SM)
                        .shadow_md()
                        .on_mouse_down_out(
                            cx.listener(|this, _, window, cx| this.close_overlays(window, cx)),
                        )
                        .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, window, cx| {
                            if ev.keystroke.key == "escape" {
                                this.close_overlays(window, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .children(body),
                ),
        )
        .priority(1);
        match at {
            Some(_) => card.into_any_element(),
            None => div()
                .absolute()
                .top_full()
                .right_0()
                .child(card)
                .into_any_element(),
        }
    }

    /// The sidebar settings popover: Unreads section, sort and density (the
    /// Appearance page's options, applied live) and "Organise sidebar".
    fn settings_popover(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        use crate::settings_controls::{segment, segmented_group, switch};
        let at = self.settings_at?;
        let colors = *cx.theme().colors();
        let row = |label: String, control: gpui::AnyElement| {
            div()
                .px(themes::metrics::SPACE_3)
                .py(themes::metrics::SPACE_2)
                .flex()
                .items_center()
                .justify_between()
                .gap(themes::metrics::SPACE_6)
                .child(label)
                .child(control)
                .into_any_element()
        };
        let title = t!(cx, "sidebar-settings").into_owned();
        let unreads = crate::controls::on_activate(
            switch("sidebar-settings-unreads", self.show_unreads, &colors)
                .role(gpui::Role::Switch)
                .aria_toggled(if self.show_unreads {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .aria_label(t!(cx, "sidebar-settings-unreads").into_owned()),
            cx,
            |this, _, cx| this.set_option(SidebarOption::ShowUnreads(!this.show_unreads), cx),
        );
        let sort_seg = |id: &'static str, label: String, v: SidebarSort, cx: &mut Context<Self>| {
            crate::controls::on_activate(
                segment(id, label, self.sort == v, &colors),
                cx,
                move |this, _, cx| this.set_option(SidebarOption::Sort(v), cx),
            )
        };
        let sort = segmented_group(
            "sidebar-settings-sort",
            t!(cx, "sidebar-settings-sort").into_owned(),
            [
                sort_seg(
                    "sidebar-settings-sort-alpha",
                    t!(cx, "sidebar-settings-sort-alpha").into_owned(),
                    SidebarSort::Alphabetical,
                    cx,
                ),
                sort_seg(
                    "sidebar-settings-sort-recent",
                    t!(cx, "sidebar-settings-sort-recent").into_owned(),
                    SidebarSort::Recent,
                    cx,
                ),
            ],
            &colors,
        );
        let density_seg =
            |id: &'static str, label: String, v: settings::Density, cx: &mut Context<Self>| {
                crate::controls::on_activate(
                    segment(id, label, self.density == v, &colors),
                    cx,
                    move |this, _, cx| this.set_option(SidebarOption::Density(v), cx),
                )
            };
        let density = segmented_group(
            "sidebar-settings-density",
            t!(cx, "sidebar-settings-density").into_owned(),
            [
                density_seg(
                    "sidebar-settings-density-compact",
                    t!(cx, "sidebar-settings-density-compact").into_owned(),
                    settings::Density::Compact,
                    cx,
                ),
                density_seg(
                    "sidebar-settings-density-cozy",
                    t!(cx, "sidebar-settings-density-cozy").into_owned(),
                    settings::Density::Cozy,
                    cx,
                ),
            ],
            &colors,
        );
        let organise = crate::controls::on_activate(
            crate::controls::button_small(
                "sidebar-settings-organise",
                t!(cx, "sidebar-settings-organise").into_owned(),
                crate::controls::ButtonKind::Secondary,
                &colors,
            )
            .debug_selector(|| "sidebar-settings-organise".to_string()),
            cx,
            |this, window, cx| this.set_organising(true, window, cx),
        );
        let body = vec![
            div()
                .px(themes::metrics::SPACE_3)
                .py(themes::metrics::SPACE_2)
                .text_size(themes::metrics::FONT_SIZE_LG)
                .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                .child(title.clone())
                .into_any_element(),
            row(
                t!(cx, "sidebar-settings-unreads").into_owned(),
                unreads.into_any_element(),
            ),
            row(
                t!(cx, "sidebar-settings-sort").into_owned(),
                sort.into_any_element(),
            ),
            row(
                t!(cx, "sidebar-settings-density").into_owned(),
                density.into_any_element(),
            ),
            div()
                .my(themes::metrics::SPACE_2)
                .h(px(1.0))
                .bg(colors.border)
                .into_any_element(),
            div()
                .px(themes::metrics::SPACE_3)
                .py(themes::metrics::SPACE_2)
                .flex()
                .child(organise)
                .into_any_element(),
        ];
        Some(self.overlay(
            "sidebar-settings-popover",
            Some(at),
            gpui::Role::Dialog,
            title,
            body,
            cx,
        ))
    }

    /// A custom section's menu: Rename, Move up, Move down, Delete section.
    fn section_menu_view(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        use crate::SectionOp;
        let (id, at) = self.section_menu?;
        let pos = self.custom_order.iter().position(|c| *c == id)?;
        let last = self.custom_order.len() - 1;
        let name = self
            .rows
            .iter()
            .find_map(|r| match r {
                SidebarRow::Header {
                    section: SidebarSection::Custom(s),
                    label,
                    ..
                } if *s == id => Some(label.to_string()),
                _ => None,
            })
            .unwrap_or_default();
        let theme = cx.theme().clone();
        let danger = theme.status().error;
        let mut body = vec![
            div()
                .px(themes::metrics::SPACE_4)
                .py(themes::metrics::SPACE_2)
                .text_size(themes::metrics::FONT_SIZE_XS)
                .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(parla_tokens(&theme).text_subtle)
                .child(t!(cx, "sidebar-section-menu-title", name = name.clone()).into_owned())
                .into_any_element(),
        ];
        let entry = |sel: &'static str,
                     icon: crate::icon::IconName,
                     label: String,
                     tint: Option<gpui::Hsla>,
                     cx: &mut Context<Self>,
                     f: MenuAction<S>| {
            crate::controls::on_activate(
                crate::notify_menu::icon_item_tinted(sel, icon, label, None, false, tint, cx)
                    .role(gpui::Role::MenuItem)
                    .tab_stop(true)
                    .focusable(),
                cx,
                move |this, window, cx| {
                    f(this, window, cx);
                },
            )
            .into_any_element()
        };
        body.push(entry(
            "sidebar-section-rename",
            crate::icon::IconName::Pencil,
            t!(cx, "sidebar-section-rename").into_owned(),
            None,
            cx,
            Box::new(move |this, window, cx| this.start_rename(id, window, cx)),
        ));
        if pos > 0 {
            body.push(entry(
                "sidebar-section-move-up",
                crate::icon::IconName::ChevUp,
                t!(cx, "sidebar-section-move-up").into_owned(),
                None,
                cx,
                Box::new(move |this, window, cx| {
                    this.close_overlays(window, cx);
                    this.section_op(SectionOp::Reorder { id, to: pos - 1 }, cx);
                }),
            ));
        }
        if pos < last {
            body.push(entry(
                "sidebar-section-move-down",
                crate::icon::IconName::ChevDown,
                t!(cx, "sidebar-section-move-down").into_owned(),
                None,
                cx,
                Box::new(move |this, window, cx| {
                    this.close_overlays(window, cx);
                    this.section_op(SectionOp::Reorder { id, to: pos + 1 }, cx);
                }),
            ));
        }
        body.push(
            div()
                .my(themes::metrics::SPACE_2)
                .h(px(1.0))
                .bg(theme.colors().border)
                .into_any_element(),
        );
        body.push(entry(
            "sidebar-section-delete",
            crate::icon::IconName::Trash,
            t!(cx, "sidebar-section-delete").into_owned(),
            Some(danger),
            cx,
            Box::new(move |this, window, cx| {
                this.close_overlays(window, cx);
                this.section_op(SectionOp::Delete { id }, cx);
            }),
        ));
        Some(self.overlay("sidebar-section-menu", at, gpui::Role::Menu, name, body, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_wins_over_muted() {
        assert_eq!(row_tone(true, false, true), RowTone::Strong);
        assert_eq!(row_tone(false, true, true), RowTone::Muted);
        assert_eq!(row_tone(false, true, false), RowTone::Strong);
        assert_eq!(row_tone(false, false, false), RowTone::Normal);
    }

    #[test]
    fn a_draft_row_uses_an_icon_not_a_pencil_glyph() {
        let text = row_text(ConversationKind::Channel, "general");
        assert!(
            !text.contains('\u{270E}'),
            "pencil must be an icon, not row text: {text:?}"
        );
        assert!(text.contains("general"));
    }
}
