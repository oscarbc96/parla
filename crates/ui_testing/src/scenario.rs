//! Named `FakeProvider` scenarios: the data behind every visual baseline and
//! end-to-end script. Building one needs no GPUI context. The names are the
//! baseline names, so they are stable, unique and snake_case.

use std::sync::Arc;

use crate::source::RichExt;
use domain::test_support::{FakeProvider, cid, cmid, conv, member, mid, ts, uid};
use domain::{
    Block, ConversationId, ConversationKind, CustomEmoji, DeliveryState, EmojiRef, FileId,
    FileKind, FileRef, ImageRef, Inline, Message, MessageTs, ProviderExt, Reaction, RemoteUrl,
    RichText, Style,
};

/// A banner shown above the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Banner {
    SessionExpired,
    /// The offline strip with Retry now.
    Offline,
    ConfigError,
    /// The incoming-huddle banner (Join and Decline).
    IncomingHuddle,
}

/// An unsent composer draft.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub text: String,
    /// The draft edits an existing message instead of composing a new one.
    pub editing: bool,
    /// Bytes of `text` selected, with the composer focused.
    pub select: Option<std::ops::Range<usize>>,
    /// Text an input method is still composing after `text` (underlined), composer focused.
    pub composing: Option<String>,
    /// The text arrives as typing, so `@`, `#` and `:` pickers open on it.
    pub typed: bool,
    /// The URL field of the link flow is open on the selection.
    pub link_prompt: bool,
    /// The Send later menu is open over the text.
    pub send_later: Option<SendLaterStage>,
    /// The emoji picker of the composer's emoji button is open.
    pub emoji_picker: bool,
    /// With `emoji_picker`: open it on the Custom tab.
    pub emoji_picker_custom: bool,
    /// This long text was just pasted: the snippet offer is open on it.
    pub snippet_paste: Option<String>,
}

/// How far the Send later flow is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendLaterStage {
    /// The presets and "Custom...".
    Menu,
    /// The custom fields, filled with a time that has passed.
    CustomPast,
}

/// Scheduled messages, and the Scheduled view beside the timeline.
#[derive(Debug, Clone, Default)]
pub struct Scheduled {
    pub server: Vec<domain::ScheduledMessage>,
    pub local: Vec<domain::LocalSchedule>,
    /// The Scheduled view is open in the right column.
    pub view: bool,
    /// Open the inline editor of this provider-side item.
    pub edit: Option<&'static str>,
}

/// Reminders, and the Reminders view beside the timeline.
#[derive(Debug, Clone, Default)]
pub struct Reminders {
    /// `None` is the loading state.
    pub items: Option<Vec<domain::Reminder>>,
    /// Parla keeps them itself (the provider has none).
    pub local: bool,
    /// The Reminders view is open in the right column.
    pub view: bool,
    /// Open the inline editor of this reminder.
    pub edit: Option<&'static str>,
    /// Open the snooze menu of this reminder.
    pub snooze: Option<&'static str>,
}

/// Saved drafts, and the Drafts view beside the timeline.
#[derive(Debug, Clone, Default)]
pub struct Drafts {
    pub items: Vec<components::DraftItem>,
    /// The Drafts view is open in the right column.
    pub view: bool,
}

/// What a collections scenario opens over the workspace.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CollectionsShow {
    #[default]
    Nothing,
    /// The pins popover under the header.
    Pins,
    /// The Later view in the centre column.
    Later,
    /// The Threads view in the centre column.
    Threads,
}

/// Pins, bookmarks, saved items and the Threads inbox (the provider has them).
#[derive(Debug, Clone, Default)]
pub struct Collections {
    pub pins: Option<Vec<domain::Pin>>,
    pub bookmarks: Option<Vec<domain::Bookmark>>,
    pub saved: Option<domain::Page<domain::SavedItem>>,
    pub inbox: Option<domain::Page<domain::InboxThread>>,
    /// Messages the lists show that the open conversation does not hold.
    pub held: Vec<Message>,
    pub show: CollectionsShow,
    /// The message actions menu is open on this message.
    pub menu: Option<MessageTs>,
}

/// The profile panel (and the status dialog) of a scenario.
#[derive(Debug, Clone)]
pub struct ProfileStage {
    pub profile: domain::Profile,
    /// A direct message with the person exists (the "Message" button).
    pub dm: bool,
    /// The person's do-not-disturb pause ends this many seconds after now.
    pub dnd_secs: Option<i64>,
    pub huddle: bool,
    /// Own presence setting.
    pub presence_setting: Option<domain::PresenceSetting>,
    /// The status dialog is open over the window.
    pub dialog: bool,
    /// The dialog has "Custom..." chosen with a custom value field.
    pub dialog_custom: bool,
    /// The dialog has this preset applied.
    pub dialog_preset: Option<usize>,
}

/// A cross-workspace view (All unreads, mentions or scheduled) filling the
/// centre column, over the scenario's rail.
#[derive(Clone)]
pub struct Aggregate {
    pub kind: components::AggregateKind,
    pub unreads: Vec<components::UnreadItem>,
    pub mentions: Vec<components::MentionItem>,
    pub scheduled: Vec<components::ScheduledItem>,
    /// "Mark all read" was clicked and waits for its confirmation.
    pub confirm: bool,
    /// Unread previews by conversation.
    pub previews: Vec<(domain::ConversationId, Vec<components::PreviewLine>)>,
}

/// State of an upload chip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UploadState {
    /// Attached, waiting for Send.
    Ready,
    /// Being sent, no percentage (the provider reports only done or failed).
    Uploading,
    /// Fraction sent, 0.0 to 1.0.
    Progress(f32),
    Failed,
    Cancelled,
}

/// A staged state of the email-code sign-in. With
/// `sign_in`, the screen has a code sign-in and is driven to this state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInStage {
    Email,
    EmailInvalid,
    EmailSending,
    EmailNeedsBrowser,
    EmailRateLimited,
    EmailOffline,
    /// The code step with Resend offered.
    Code,
    CodePartial,
    CodeVerifying,
    CodeWrong,
    CodeExpired,
    CodeTooMany,
    CodeRateLimited,
    /// The code step while Resend waits.
    CodeResendWait,
    CodeResent,
}

/// How far a first page of messages or replies got.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LoadStage {
    /// Arrived (the default).
    #[default]
    Loaded,
    /// Still in flight.
    Loading,
    /// The request failed.
    Failed,
}

/// One scenario: a scripted provider plus what the UI shows on top of it.
/// A channel admin dialog over the window, in a given state.
#[derive(Clone, Debug)]
pub enum AdminStage {
    /// Create a channel with `name` typed; `error` is the provider's answer.
    Create {
        name: &'static str,
        private: bool,
        error: Option<&'static str>,
    },
    /// The channel browser with these rows.
    Browse(Vec<components::BrowseRow>),
    /// The browser waiting for its first page.
    BrowseLoading,
    /// The browser with no channel for the search.
    BrowseEmpty,
    /// The browser after the provider refused the page.
    BrowseError(&'static str),
    /// Edit the open channel: name, topic, description.
    Edit {
        name: &'static str,
        topic: &'static str,
        purpose: &'static str,
    },
    /// Add people: the candidates and the ones ticked.
    Invite {
        title: &'static str,
        people: Vec<components::InviteRow>,
        picked: Vec<usize>,
    },
    /// The leave or archive confirm.
    Confirm {
        title: &'static str,
        body: &'static str,
        ok: &'static str,
        /// Archive: the confirm button in the danger colour.
        danger: bool,
    },
}

pub struct Scenario {
    pub name: &'static str,
    pub provider: Arc<FakeProvider>,
    /// Show the sign-in screen instead of a workspace.
    pub sign_in: bool,
    /// With `sign_in`: the paste form is open, its first field focused.
    pub sign_in_form: bool,
    /// With `sign_in`: a sign-in ran and failed, so the error shows.
    pub sign_in_error: bool,
    /// With `sign_in`: provider strings about 40% longer, for the pseudo-locale goldens.
    pub sign_in_long: bool,
    /// With `sign_in`: the email-code sign-in, driven to this state.
    pub sign_in_stage: Option<SignInStage>,
    /// Show the native workspace picker instead of a workspace.
    pub workspace_picker: Option<Vec<components::workspace_picker::WorkspaceChoice>>,
    pub open: Option<ConversationId>,
    pub banner: Option<Banner>,
    /// Root of the open thread panel.
    pub thread: Option<MessageTs>,
    /// The provider can follow threads and the open thread is followed (or not).
    pub followed: Option<bool>,
    /// The open conversation's first history page.
    pub history: LoadStage,
    /// The open thread's replies.
    pub replies: LoadStage,
    /// Message selected in the timeline (as after a click on it).
    pub selected: Option<MessageTs>,
    pub draft: Option<Draft>,
    pub uploads: Vec<(String, UploadState)>,
    /// Snippet cards (by file id) shown expanded.
    pub expanded_snippets: Vec<&'static str>,
    /// A clip card with its decoder open and paused, showing the controls.
    pub clip: Option<StagedClip>,
    /// The sidebar's "Direct messages" section is expanded.
    pub dms_expanded: bool,
    /// Draw the workspace rail and the conversation header too.
    pub chrome: Option<Chrome>,
    /// The provider can schedule; these items exist (the sidebar shows "Scheduled").
    pub scheduled: Option<Scheduled>,
    /// The sidebar shows "Reminders" and these items exist.
    pub reminders: Option<Reminders>,
    /// Saved drafts (the sidebar shows "Drafts" while any exist).
    pub saved_drafts: Option<Drafts>,
    /// The provider serves custom emoji and the workspace uses them.
    pub custom_emoji_capability: bool,
    /// Sidebar sections the user collapsed.
    pub collapsed: Vec<components::SidebarSection>,
    /// The user's own sidebar sections (`sidebar.sections`).
    pub sidebar_sections: Vec<settings::CustomSection>,
    /// The sidebar is in organise mode.
    pub sidebar_organise: bool,
    /// The sidebar settings popover is open.
    pub sidebar_settings: bool,
    /// The menu of this custom section is open.
    pub section_menu: Option<u32>,
    /// The provider has pins, bookmarks, saved items and the Threads inbox.
    pub collections: Option<Collections>,
    /// The search panel is open in the right column.
    pub search: Option<SearchStage>,
    /// The timeline is detached around this message (a jump): scrolled to it,
    /// highlighted, with the "jump to latest" bar.
    pub jump: Option<MessageTs>,
    /// A cross-workspace view replaces the conversation.
    pub aggregate: Option<Aggregate>,
    /// Notification levels and a pause the workspace carries.
    pub notify: Option<NotifyStage>,
    /// Conversations the user starred (Parla's own list).
    pub starred: Vec<ConversationId>,
    /// The profile panel is open in the right column.
    pub profile: Option<ProfileStage>,
    /// Profiles the store already holds (no panel open).
    pub profiles: Vec<domain::Profile>,
    /// The channel details panel is open in the right column on this tab.
    pub details: Option<components::DetailsTab>,
    /// A channel admin dialog is open over the window.
    pub admin: Option<AdminStage>,
    /// Whether each person on screen is active (the presence dots).
    pub presence: Vec<(domain::UserId, bool)>,
    /// People typing in the open conversation (the line above the composer).
    pub typing: Vec<domain::UserId>,
    /// Live huddles (the provider has huddles: bar, sidebar marks, call cards).
    pub huddles: Vec<domain::Huddle>,
    /// The call bar for the running call (the open conversation's huddle).
    pub call: Option<components::CallBarModel>,
    /// The provider can start huddles: the bar offers Start where none runs.
    pub start_huddle: bool,
    /// The realtime connection is down (unsent messages show "Queued").
    pub offline: bool,
    /// The session expired (the composer is read-only).
    pub expired: bool,
    /// The People directory is open in the right column.
    pub people: Option<PeopleStage>,
    /// The open conversation shows its Files tab instead of the messages.
    pub files_tab: bool,
}

/// What the People directory shows in a scenario.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PeopleStage {
    /// Typed in its search field.
    pub query: &'static str,
}

/// Notification controls state of a scenario.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NotifyStage {
    /// Parla's own levels per conversation (the provider keeps none).
    pub levels: Vec<(ConversationId, domain::ConversationPrefs)>,
    /// Parla's own extras per conversation (thread replies, sound, timed mute).
    pub extras: Vec<(ConversationId, domain::NotifyExtras)>,
    /// The user's own pause ends this many seconds after the scripted now.
    pub paused_for: Option<i64>,
}

/// What the search panel shows in a scenario.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchStage {
    pub query: String,
    pub results: components::SearchResults,
    /// The provider searches on its server (scope tabs).
    pub remote: bool,
}

/// The rail and header of a workspace scenario.
#[derive(Debug, Clone, PartialEq)]
pub struct Chrome {
    /// The workspace icon on the rail; initials when it is `None`.
    pub icon: Option<ImageRef>,
    /// The rail's entries; one workspace with `icon` when `None`.
    pub rail: Option<Arc<[components::rail::RailItem]>>,
    /// The rail ends with Settings and the signed-in user's avatar.
    pub me: bool,
}

const SELF: &str = "u1";
/// Base of the scripted clock (Unix seconds); every message is relative to it.
const T0: u64 = 1_700_000_000;
const DAY: u64 = 86_400;
/// The scripted "now" (Unix seconds): the day after the last scripted day.
pub const NOW_SECS: u64 = T0 + 2 * DAY + 3_600;

fn at(secs: u64, micros: u32) -> String {
    format!("{secs}.{micros:06}")
}

fn base() -> FakeProvider {
    let p = FakeProvider::new("w1", uid(SELF));
    p.set_workspace_name("Example Workspace");
    p.add_member(member(SELF, "Me"));
    for (id, name) in [
        ("u2", "Ana Reyes"),
        ("u3", "Ben Okafor"),
        ("u4", "Cy Lindqvist"),
    ] {
        p.add_member(member(id, name));
    }
    p
}

fn from(c: &str, t: &str, author: &str, text: &str) -> Message {
    Message::new(mid(c, t), Some(uid(author)), Arc::from(text))
}

fn emoji(shortcode: &str, unicode: &str) -> EmojiRef {
    EmojiRef {
        shortcode: Arc::from(shortcode),
        unicode: Some(Arc::from(unicode)),
        skin_tone: None,
    }
}

fn reaction(shortcode: &str, unicode: &str, users: &[&str]) -> Reaction {
    Reaction {
        emoji: emoji(shortcode, unicode),
        count: users.len() as u32,
        users: users.iter().map(|u| uid(u)).collect(),
        me: users.contains(&SELF),
    }
}

fn file(id: &str, name: &str, mime: &str, size: u64, kind: FileKind) -> FileRef {
    FileRef {
        id: FileId(Arc::from(id)),
        name: Arc::from(name),
        title: None,
        mime: Arc::from(mime),
        size,
        kind,
        thumbs: Arc::from([]),
        download: None,
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

fn scenario(name: &'static str, provider: FakeProvider) -> Scenario {
    Scenario {
        name,
        provider: Arc::new(provider),
        sign_in: false,
        sign_in_form: false,
        sign_in_error: false,
        sign_in_long: false,
        sign_in_stage: None,
        workspace_picker: None,
        open: None,
        banner: None,
        thread: None,
        followed: None,
        history: LoadStage::Loaded,
        replies: LoadStage::Loaded,
        selected: None,
        draft: None,
        uploads: Vec::new(),
        expanded_snippets: Vec::new(),
        clip: None,
        dms_expanded: false,
        chrome: None,
        scheduled: None,
        reminders: None,
        saved_drafts: None,
        custom_emoji_capability: false,
        collapsed: Vec::new(),
        sidebar_sections: Vec::new(),
        sidebar_organise: false,
        sidebar_settings: false,
        section_menu: None,
        collections: None,
        search: None,
        jump: None,
        aggregate: None,
        notify: None,
        starred: Vec::new(),
        profile: None,
        profiles: Vec::new(),
        details: None,
        admin: None,
        presence: Vec::new(),
        typing: Vec::new(),
        huddles: Vec::new(),
        call: None,
        start_huddle: false,
        offline: false,
        expired: false,
        people: None,
        files_tab: false,
    }
}

/// A workspace with `#general` open and the given messages in it.
fn general(name: &'static str, messages: Vec<Message>) -> Scenario {
    let p = base();
    p.add_conversation(conv("C1", "general"));
    p.add_messages(&cid("C1"), messages);
    let mut s = scenario(name, p);
    s.open = Some(cid("C1"));
    s
}

/// A live huddle `mins` minutes before the scripted now.
fn live_huddle(c: &str, who: &[&str], mins: u64) -> domain::Huddle {
    domain::Huddle {
        id: domain::HuddleId(Arc::from(format!("H-{c}"))),
        conversation: cid(c),
        participants: who.iter().map(|u| uid(u)).collect::<Vec<_>>().into(),
        started: (NOW_SECS - mins * 60) as i64,
        created_by: who.first().map(|u| uid(u)),
        is_dm: false,
    }
}

/// A message whose body is one call card.
fn call_message(t: &str, author: &str, plain: &str, card: domain::CallCard) -> Message {
    let mut m = from("C1", t, author, plain);
    m.ext = ProviderExt::new(RichExt(RichText {
        blocks: Arc::from(vec![Block::Call(card)]),
    }));
    m
}

/// The huddle bar, the sidebar mark and the three kinds of call card.
fn huddle_awareness(name: &'static str, self_in: bool) -> Scenario {
    let who: &[&str] = if self_in {
        &["u1", "u2", "u3"]
    } else {
        &["u2", "u3", "u4", "u5", "u6", "u7"]
    };
    let started = (NOW_SECS - 12 * 60) as i64;
    let people = |ids: &[&str]| -> Arc<[domain::UserId]> {
        ids.iter().map(|u| uid(u)).collect::<Vec<_>>().into()
    };
    let msgs = vec![
        call_message(
            &at(T0 + 60, 1),
            "u2",
            "Huddle",
            domain::CallCard {
                kind: domain::CallKind::Huddle,
                title: None,
                join_url: None,
                started: Some(T0 as i64 + 60),
                ended: Some(T0 as i64 + 60 + 15 * 60),
                participants: people(&["u2", "u3"]),
                created_by: Some(uid("u2")),
            },
        ),
        call_message(
            &at(T0 + 120, 1),
            "u4",
            "Call: Meetly",
            domain::CallCard {
                kind: domain::CallKind::External {
                    provider: Arc::from("Meetly"),
                },
                title: Some(Arc::from("Design review")),
                join_url: Some(Arc::from("https://meet.example.com/design-review")),
                started: None,
                ended: None,
                participants: people(&[]),
                created_by: None,
            },
        ),
        call_message(
            &at(T0 + 180, 1),
            "u2",
            "Huddle",
            domain::CallCard {
                kind: domain::CallKind::Huddle,
                title: None,
                join_url: None,
                started: Some(started),
                ended: None,
                participants: people(who),
                created_by: Some(uid("u3")),
            },
        ),
    ];
    let mut s = general(name, msgs);
    for (id, n) in [("u5", "Dee Park"), ("u6", "Eli Ford"), ("u7", "Fay Gray")] {
        s.provider.add_member(member(id, n));
    }
    s.provider.add_conversation(conv("C2", "random"));
    s.huddles = vec![
        live_huddle("C1", who, 12),
        live_huddle("C2", &["u3", "u4"], 3),
    ];
    s.chrome = Some(Chrome {
        me: false,
        icon: None,
        rail: None,
    });
    s
}

/// The call bar over a live huddle the user is in: connecting (with the slow
/// load offer and no mute control) or live (mute and camera state).
fn call_bar(name: &'static str, live: bool) -> Scenario {
    use components::{CallBarModel, CallBarPhase, MediaState};
    let mut s = general(name, Vec::new());
    s.provider.add_member(member("u2", "Bob Reyes"));
    s.huddles = vec![live_huddle("C1", &["u1", "u2"], 4)];
    s.call = Some(CallBarModel {
        title: "general".into(),
        phase: if live {
            CallBarPhase::Live
        } else {
            CallBarPhase::Loading
        },
        mic: if live {
            MediaState::Muted
        } else {
            MediaState::Unsupported
        },
        camera: if live {
            MediaState::Active
        } else {
            MediaState::Unsupported
        },
        participants: 2,
        load_slow: !live,
        screen_share_unavailable: false,
        show_window: true,
        people: Vec::new(),
        devices: None,
    });
    s.chrome = Some(Chrome {
        me: false,
        icon: None,
        rail: None,
    });
    s
}

/// A live native call: the people as avatars with the speaker ringed and
/// named, a share Parla cannot show (Open in browser) and the device picker,
/// closed or `open`.
fn call_bar_native(name: &'static str, open: bool) -> Scenario {
    use components::{CallBarModel, CallBarPhase, CallPerson, DeviceMenu, MediaState};
    let mut s = general(name, Vec::new());
    for (id, n) in [
        ("u2", "Bob Reyes"),
        ("u3", "Cleo Diaz"),
        ("u4", "Dev Patel"),
    ] {
        s.provider.add_member(member(id, n));
    }
    s.huddles = vec![live_huddle("C1", &["u1", "u2", "u3", "u4"], 4)];
    let person = |name: &str, speaking: bool| CallPerson {
        name: name.to_string().into(),
        speaking,
        muted: name == "Dev Patel",
    };
    s.call = Some(CallBarModel {
        title: "general".into(),
        phase: CallBarPhase::Live,
        mic: MediaState::Active,
        camera: MediaState::Unsupported,
        participants: 4,
        load_slow: false,
        screen_share_unavailable: !open,
        show_window: false,
        people: vec![
            person("Bob Reyes", true),
            person("Cleo Diaz", false),
            person("Dev Patel", false),
        ],
        devices: Some(DeviceMenu {
            open,
            inputs: vec!["Built-in Microphone".into(), "USB Mic".into()],
            outputs: vec!["Built-in Speakers".into(), "Headphones".into()],
            input: Some("USB Mic".into()),
            output: None,
        }),
    });
    s.chrome = Some(Chrome {
        me: false,
        icon: None,
        rail: None,
    });
    s
}

/// No huddle runs: the bar offers Start huddle.
fn huddle_start() -> Scenario {
    let mut s = general("huddle_start", Vec::new());
    s.start_huddle = true;
    s.chrome = Some(Chrome {
        me: false,
        icon: None,
        rail: None,
    });
    s
}

fn sign_in_form() -> Scenario {
    let mut s = scenario("sign_in_form", FakeProvider::new("w0", uid(SELF)));
    s.sign_in = true;
    s.sign_in_form = true;
    s
}

fn sign_in_long() -> Scenario {
    let mut s = scenario("sign_in_long", FakeProvider::new("w0", uid(SELF)));
    s.sign_in = true;
    s.sign_in_long = true;
    s
}

fn sign_in_form_long() -> Scenario {
    let mut s = scenario("sign_in_form_long", FakeProvider::new("w0", uid(SELF)));
    s.sign_in = true;
    s.sign_in_form = true;
    s.sign_in_long = true;
    s
}

fn sign_in_error() -> Scenario {
    let mut s = scenario("sign_in_error", FakeProvider::new("w0", uid(SELF)));
    s.sign_in = true;
    s.sign_in_error = true;
    s
}

fn sign_in() -> Scenario {
    let mut s = scenario("sign_in", FakeProvider::new("w0", uid(SELF)));
    s.sign_in = true;
    s
}

/// The native list after a sign-in found three workspaces: one to add, one
/// that needs SSO, and one already in Parla.
fn workspace_picker() -> Scenario {
    use domain::ChatProvider as _;
    let p = FakeProvider::new("w1", uid(SELF));
    p.set_workspace_name("Example Workspace");
    let added = p.workspace().id.0.to_string();
    let mut s = scenario("workspace_picker", p);
    let row = |id: &str, name: &str, domain: &str, sso: bool, added: bool, updating: bool| {
        components::workspace_picker::WorkspaceChoice {
            id: id.into(),
            name: name.into(),
            domain: domain.into(),
            icon: None,
            sso,
            two_factor: false,
            signed_out: false,
            added,
            updated: added && !updating,
            updating,
            update_error: false,
            needs_sign_in: false,
        }
    };
    s.workspace_picker = Some(vec![
        row("T0101", "Acme Corp", "acme.example", false, false, false),
        row("T0102", "Globex Labs", "globex.example", true, false, false),
        row(
            &added,
            "Example Workspace",
            "example.example",
            false,
            true,
            true,
        ),
    ]);
    s
}

/// Presence dots: the sidebar's direct messages, the open direct message's
/// header and the profile panel, one person active and one away.
fn presence_dots() -> Scenario {
    let mut s = sidebar_busy();
    s.name = "presence_dots";
    s.draft = None;
    s.open = Some(cid("D1"));
    s.chrome = Some(Chrome {
        me: false,
        icon: None,
        rail: None,
    });
    s.provider.add_member(member("u2", "Ana Reyes"));
    s.provider.add_member(member("u3", "Ben Okafor"));
    let mut p = profile_of("u2", "Ana Reyes");
    p.title = Some(Arc::from("Staff engineer"));
    p.tz = Some(Arc::from("Asia/Kolkata"));
    s.profile = Some(ProfileStage {
        profile: p,
        dm: true,
        dnd_secs: None,
        huddle: false,
        presence_setting: None,
        dialog: false,
        dialog_custom: false,
        dialog_preset: None,
    });
    s.presence = vec![(uid("u2"), true), (uid("u3"), false)];
    s
}

/// The sidebar with a Starred section (a channel and a direct message) above
/// Channels and Direct messages; the starred channel is muted.
fn sidebar_starred() -> Scenario {
    let mut s = sidebar_busy();
    s.name = "sidebar_starred";
    s.starred = vec![cid("C4"), cid("D1")];
    s
}

fn sidebar_busy() -> Scenario {
    let p = base();
    let mut rows = Vec::new();
    for (id, name) in [
        ("C1", "general"),
        ("C2", "design"),
        ("C3", "random"),
        ("C4", "releases"),
    ] {
        rows.push(conv(id, name));
    }
    rows[1].unread = true;
    rows[2].unread = true;
    rows[2].mentions = 3;
    rows[3].is_muted = true;
    let mut private = conv("C5", "leads");
    private.kind = ConversationKind::PrivateChannel;
    rows.push(private);
    for (id, name, peer) in [("D1", "Ana Reyes", "u2"), ("D2", "Ben Okafor", "u3")] {
        let mut d = conv(id, name);
        d.kind = ConversationKind::Direct;
        d.peer = Some(uid(peer));
        rows.push(d);
    }
    rows[5].unread = true;
    rows[5].mentions = 1;
    let mut g = conv("G1", "Ana, Ben, Cy");
    g.kind = ConversationKind::Group;
    rows.push(g);
    for c in rows {
        p.add_conversation(c);
    }
    p.add_message(from("C2", &at(T0, 1), "u2", "Updated mockups are up."));
    p.add_message(from("C3", &at(T0, 2), "u3", "@Me lunch at noon?"));
    let mut s = scenario("sidebar_busy", p);
    s.open = Some(cid("C1"));
    // Drafts are keyed by conversation in the store; the scenario flags C2's.
    s.draft = Some(Draft {
        text: "Half-written reply".into(),
        ..Default::default()
    });
    s
}

fn timeline_rich() -> Scenario {
    let day1 = T0;
    let day2 = T0 + DAY;
    let mut edited = from(
        "C1",
        &at(day1 + 120, 1),
        "u3",
        "Standup moved to 10:30 (edited)",
    );
    edited.edited = Some(ts(&at(day1 + 180, 1)));
    general(
        "timeline_rich",
        vec![
            from(
                "C1",
                &at(day1, 1),
                "u2",
                "Morning all. Release notes draft is ready.",
            ),
            from(
                "C1",
                &at(day1 + 30, 1),
                "u2",
                "Second message in the same group.",
            ),
            edited,
            from(
                "C1",
                &at(day2, 1),
                "u4",
                "Run `mise run test` before pushing, then see https://example.com/docs/testing for details.",
            ),
            from("C1", &at(day2 + 60, 1), "u2", "@Me can you review this?"),
            from(
                "C1",
                &at(day2 + 90, 1),
                "u3",
                "```\nfn main() {\n    println!(\"hello\");\n}\n```",
            ),
        ],
    )
}

/// Opened with unread messages: a day divider per day and the "New" divider
/// above the first message after the read marker.
fn timeline_unread_divider() -> Scenario {
    let yesterday = T0 + DAY;
    let today = T0 + 2 * DAY;
    let p = base();
    let mut c = conv("C1", "general");
    c.unread = true;
    c.last_read = Some(ts(&at(today, 1)));
    p.add_conversation(c);
    p.add_messages(
        &cid("C1"),
        vec![
            from("C1", &at(yesterday, 1), "u2", "Release branch is cut."),
            from(
                "C1",
                &at(yesterday + 600, 1),
                "u3",
                "Changelog looks good to me.",
            ),
            from(
                "C1",
                &at(today, 1),
                "u2",
                "Morning. Tagging the release now.",
            ),
            from("C1", &at(today + 900, 1), "u3", "Tag is up, CI is running."),
            from("C1", &at(today + 1_500, 1), "u4", "Packages are building."),
        ],
    );
    let mut s = scenario("timeline_unread_divider", p);
    s.open = Some(cid("C1"));
    s
}

fn text(s: &str, bold: bool, italic: bool, strike: bool) -> Inline {
    Inline::Text {
        text: Arc::from(s),
        style: Style {
            bold,
            italic,
            strike,
        },
    }
}

/// A message whose formatting was parsed from provider markup (the scenario
/// stands in for the provider's parser with ready-made rich text).
fn formatted(c: &str, t: &str, author: &str, plain: &str, inlines: Vec<Inline>) -> Message {
    let mut m = from(c, t, author, plain);
    m.ext = ProviderExt::new(RichExt(RichText {
        blocks: Arc::from(vec![Block::Paragraph(inlines)]),
    }));
    m
}

/// Messages that carry only formatted text: styles, code, a mention, a channel
/// link, a labelled link and emoji, plus a custom emoji reaction.
fn timeline_markup() -> Scenario {
    let p = base();
    p.add_conversation(conv("C1", "general"));
    p.add_custom_emoji(CustomEmoji {
        shortcode: Arc::from("solved"),
        image: ImageRef::Provider(RemoteUrl(Arc::from("https://example.invalid/solved.png"))),
    });
    let mut m1 = formatted(
        "C1",
        &at(T0, 1),
        "u2",
        "*bold* _italic_ ~old~ `code`",
        vec![
            text("bold", true, false, false),
            text(" ", false, false, false),
            text("italic", false, true, false),
            text(" ", false, false, false),
            text("old", false, false, true),
            text(" ", false, false, false),
            Inline::Code(Arc::from("code")),
        ],
    );
    m1.reactions = Arc::from([
        reaction("tada", "\u{1F389}", &["u3"]),
        Reaction {
            emoji: EmojiRef {
                shortcode: Arc::from("solved"),
                unicode: None,
                skin_tone: None,
            },
            count: 1,
            users: Arc::from([uid("u4")]),
            me: false,
        },
    ]);
    let m2 = formatted(
        "C1",
        &at(T0 + 60, 1),
        "u3",
        "<@u2> see <#C1|general>",
        vec![
            Inline::User {
                id: uid("u2"),
                label: None,
            },
            text(" see ", false, false, false),
            Inline::Conversation {
                id: cid("C1"),
                label: Some(Arc::from("general")),
            },
            text(" and ", false, false, false),
            Inline::Link {
                url: Arc::from("https://example.com/p/1"),
                label: Some(Arc::from("link")),
                style: Style::default(),
            },
            text(" ", false, false, false),
            Inline::Emoji(emoji("tada", "\u{1F389}")),
            text(" ", false, false, false),
            Inline::Emoji(EmojiRef {
                shortcode: Arc::from("solved"),
                unicode: None,
                skin_tone: None,
            }),
        ],
    );
    p.add_messages(&cid("C1"), vec![m1, m2]);
    let mut s = scenario("timeline_markup", p);
    s.open = Some(cid("C1"));
    s
}

/// A bot alert as the provider parser yields it: a pretext line, then an
/// attachment card with a colour bar, paragraphs separated by one blank line
/// and compact fields.
fn timeline_bot_alert() -> Scenario {
    let t = |s: &str| text(s, false, false, false);
    let br = || Inline::LineBreak;
    let card = domain::Card {
        color: Some(0xa30200),
        author: None,
        title: Some(text("High error rate on checkout", true, false, false)),
        body: vec![
            Block::Paragraph(vec![t("Error rate is 7.2% over the last 5 minutes.")]),
            Block::Paragraph(vec![
                t("Top error: payment gateway timeout"),
                br(),
                t("Started at 10:42 UTC"),
            ]),
        ],
        fields: vec![
            (Arc::from("Service"), vec![t("checkout-api")]),
            (Arc::from("Environment"), vec![t("production")]),
            (Arc::from("Severity"), vec![t("critical")]),
        ],
        image: None,
        thumb: None,
        footer: Some(vec![t("Alert rule: checkout error budget")]),
    };
    let mut alert = from("C1", &at(T0 + 60, 1), "u5", "High error rate on checkout");
    alert.ext = ProviderExt::new(RichExt(RichText {
        blocks: Arc::from(vec![
            Block::Paragraph(vec![t("Alert firing for checkout")]),
            Block::Card(card),
        ]),
    }));
    let follow = from("C1", &at(T0 + 300, 1), "u2", "On it, checking the gateway.");
    let s = general("timeline_bot_alert", vec![alert, follow]);
    let mut bot = member("u5", "Alert Bot");
    bot.is_bot = true;
    s.provider.add_member(bot);
    s
}

fn timeline_states() -> Scenario {
    let mut pending = from("C1", &at(T0 + 10, 1), SELF, "Sending this one right now");
    pending.state = DeliveryState::Pending;
    pending.client_msg_id = Some(cmid("pending-1"));
    let mut failed = from("C1", &at(T0 + 20, 1), SELF, "This one could not be sent");
    failed.state = DeliveryState::Failed("network error".into());
    failed.client_msg_id = Some(cmid("failed-1"));
    let mut deleted = from("C1", &at(T0 + 5, 1), "u2", "");
    deleted.plain = Arc::from("");
    general(
        "timeline_states",
        vec![
            from("C1", &at(T0, 1), "u2", "A delivered message."),
            deleted,
            pending,
            failed,
        ],
    )
}

/// Offline: a message sent while the connection is down waits as "Queued".
fn timeline_offline() -> Scenario {
    let mut queued = from("C1", &at(T0 + 60, 1), SELF, "Pushing the fix now.");
    queued.state = DeliveryState::Pending;
    queued.client_msg_id = Some(cmid("queued-1"));
    let mut s = general(
        "timeline_offline",
        vec![
            from("C1", &at(T0, 1), "u2", "Is the release branch cut yet?"),
            queued,
        ],
    );
    s.offline = true;
    s.banner = Some(Banner::Offline);
    s
}

/// An expired session: the banner names the workspace and the composer is
/// read-only.
fn composer_session_expired() -> Scenario {
    let mut s = composer("composer_session_expired", None);
    s.expired = true;
    s.banner = Some(Banner::SessionExpired);
    s
}

fn timeline_selected() -> Scenario {
    let picked = at(T0 + 60, 1);
    let mut s = general(
        "timeline_selected",
        vec![
            from("C1", &at(T0, 1), "u2", "Is the release branch cut yet?"),
            from(
                "C1",
                &picked,
                "u3",
                "Yes, cut this morning. Release notes are in the usual doc.",
            ),
            from(
                "C1",
                &at(T0 + 120, 1),
                "u4",
                "Thanks, starting the smoke tests.",
            ),
        ],
    );
    s.selected = Some(ts(&picked));
    s
}

fn reactions() -> Scenario {
    let mut m = from("C1", &at(T0, 1), "u2", "Shipped the new onboarding flow.");
    m.reactions = Arc::from([
        reaction("tada", "\u{1F389}", &["u2", "u3", "u4"]),
        reaction("+1", "\u{1F44D}", &[SELF, "u3"]),
        reaction("eyes", "\u{1F440}", &["u4"]),
    ]);
    general("reactions", vec![m])
}

fn thread_open() -> Scenario {
    let root_ts = at(T0, 1);
    let mut root = from("C1", &root_ts, "u2", "Which database should the cache use?");
    root.reply_count = 2;
    root.latest_reply = Some(ts(&at(T0 + 120, 1)));
    root.reply_users = Arc::from([uid("u3"), uid("u4")]);
    let mut r1 = from(
        "C1",
        &at(T0 + 60, 1),
        "u3",
        "SQLite, it is already a dependency.",
    );
    r1.thread_root = Some(ts(&root_ts));
    let mut r2 = from(
        "C1",
        &at(T0 + 120, 1),
        "u4",
        "Agreed, one file and no server.",
    );
    r2.thread_root = Some(ts(&root_ts));
    let mut s = general("thread_open", vec![root, r1, r2]);
    s.thread = Some(ts(&root_ts));
    s
}

/// The thread of `thread_open` as the pop-out window shows it.
fn popout_thread() -> Scenario {
    let mut s = thread_open();
    s.name = "popout_thread";
    s
}

/// A search for "cache": two hits on the device, one from the server below
/// them, a note for an ignored filter, and the jump to the first hit shown in
/// the timeline behind it.
fn search_results() -> Scenario {
    let mut msgs: Vec<Message> = (0..12)
        .map(|i| {
            from(
                "C1",
                &at(T0 + i * 60, 1),
                if i % 2 == 0 { "u2" } else { "u3" },
                &format!("Update {i}: still looking at the cache layout."),
            )
        })
        .collect();
    msgs[5].text = Arc::from("Which database should the cache use?");
    let mut s = general("search_results", msgs);
    s.jump = Some(ts(&at(T0 + 5 * 60, 1)));
    let hit = |t: &str, conv: &str, who: &str, text: &str| {
        components::SearchRow::message(mid("C1", t), conv, who, "Nov 14, 2023, 22:13", text)
    };
    s.search = Some(SearchStage {
        query: "in:#general cache".into(),
        remote: true,
        results: components::SearchResults {
            query: "in:#general cache".into(),
            rows: vec![
                hit(
                    &at(T0 + 5 * 60, 1),
                    "#general",
                    "Ana Reyes",
                    "Which database should the cache use?",
                ),
                hit(
                    &at(T0 + 2 * 3_600, 1),
                    "#general",
                    "Ben Okafor",
                    "SQLite, it is already a dependency for the cache.",
                ),
            ],
            remote: vec![hit(
                &at(T0 - DAY, 1),
                "#general",
                "Cy Lindqvist",
                "Cache invalidation notes from last week.",
            )],
            warnings: vec!["from:@ghost".into()],
            ..Default::default()
        },
    });
    s
}

/// One local hit selected: the "In context" pane shows it among the held
/// messages around it, with "Jump to message".
fn search_preview() -> Scenario {
    let mut s = search_results();
    s.name = "search_preview";
    if let Some(st) = s.search.as_mut() {
        st.query = "database".into();
        st.results.query = "database".into();
        st.results.rows.truncate(1);
        st.results.remote.clear();
        st.results.warnings.clear();
    }
    s
}

/// The search panel with the device offline: local rows and the note.
fn search_offline() -> Scenario {
    let mut s = search_results();
    s.name = "search_offline";
    if let Some(st) = s.search.as_mut() {
        st.results.remote.clear();
        st.results.warnings.clear();
        st.results.offline = true;
    }
    s
}

fn composer(name: &'static str, draft: Option<Draft>) -> Scenario {
    let mut s = general(
        name,
        vec![from(
            "C1",
            &at(T0, 1),
            "u2",
            "Anything to add before we ship?",
        )],
    );
    s.draft = draft;
    s
}

/// Someone else typing in the open conversation: the line above the composer.
fn composer_typing() -> Scenario {
    let mut s = composer("composer_typing", None);
    s.typing = vec![uid("u2")];
    s
}

/// "composer_empty" shows a public channel the user is not in, opened from
/// search: its messages, and the Join strip in place of the input (the empty
/// input is in every other conversation scenario).
fn composer_join() -> Scenario {
    let p = base();
    let caps = domain::Capabilities {
        channel_admin: true,
        ..domain::ChatProvider::capabilities(&p).clone()
    };
    let p = p.with_capabilities(caps);
    let mut c = conv("C1", "general");
    c.is_member = false;
    p.add_conversation(c);
    p.add_messages(
        &cid("C1"),
        vec![from(
            "C1",
            &at(T0, 1),
            "u2",
            "Anything to add before we ship?",
        )],
    );
    let mut s = scenario("composer_empty", p);
    s.open = Some(cid("C1"));
    s
}

/// A composer with the Send later menu (or its custom fields) open.
fn composer_send_later(name: &'static str, stage: SendLaterStage, server: bool) -> Scenario {
    let mut s = composer(
        name,
        Some(Draft {
            text: "Release notes are ready for review".into(),
            send_later: Some(stage),
            ..Default::default()
        }),
    );
    if server {
        s.scheduled = Some(Scheduled::default());
    }
    s
}

/// The Scheduled view: items of the provider and of Parla, a failed one among them.
fn scheduled_view(name: &'static str, long: bool, edit: Option<&'static str>) -> Scenario {
    let item = |id: &str, text: &str, post_at: u64| domain::ScheduledMessage {
        id: domain::ScheduleId(Arc::from(id)),
        conversation: cid("C1"),
        thread: None,
        text: Arc::from(text),
        post_at: post_at as i64,
    };
    let local = |id: &str, text: &str, post_at: u64, st: domain::LocalScheduleState| {
        domain::LocalSchedule {
            id: domain::ScheduleId(Arc::from(id)),
            conversation: cid("C1"),
            thread: None,
            text: Arc::from(text),
            files: Vec::new(),
            post_at: post_at as i64,
            created_ms: 0,
            state: st,
        }
    };
    let mut s = composer(name, None);
    s.scheduled = Some(Scheduled {
        server: vec![
            item(
                "S1",
                "Standup notes go out first thing",
                NOW_SECS + 10 * 3_600,
            ),
            item("S2", "Reminder: freeze starts Monday", NOW_SECS + 2 * DAY),
        ],
        local: vec![
            local(
                "L1",
                "Design review slides attached",
                NOW_SECS + 10 * 3_600 + 1_800,
                domain::LocalScheduleState::Pending,
            ),
            local(
                "L2",
                "Did this one go out?",
                NOW_SECS - 600,
                domain::LocalScheduleState::Failed("network error".into()),
            ),
        ],
        view: true,
        edit,
    });
    if long {
        // Pseudo-locale lengths: long texts and tags must wrap inside their cards.
        if let Some(sc) = &mut s.scheduled {
            for m in &mut sc.server {
                m.text = Arc::from(format!("{} {}", m.text, LONG_TEXT));
            }
            for m in &mut sc.local {
                m.text = Arc::from(format!("{} {}", m.text, LONG_TEXT));
            }
            if let Some(m) = sc.local.last_mut() {
                m.state = domain::LocalScheduleState::Failed(
                    "the connection was reset by the peer while sending the message".into(),
                );
            }
        }
    }
    s
}

/// Long text for the pseudo-locale style scenarios.
const LONG_TEXT: &str = "and keep going with a much longer sentence that has to wrap onto several lines without pushing the buttons out of the card";

/// The Drafts view as on the board: channel, direct message, thread and an
/// older channel draft, newest first; `empty` shows the empty state.
fn drafts_view(name: &'static str, empty: bool) -> Scenario {
    use components::{DraftItem, DraftKind};
    let item = |c: &str, thread: Option<&str>, label: &str, kind, text: &str, ago: u64| DraftItem {
        key: domain::DraftKey {
            conversation: cid(c),
            thread: thread.map(|t| MessageTs(Arc::from(t))),
        },
        conversation: label.to_string(),
        kind,
        text: text.to_string(),
        edited: (NOW_SECS - ago) as i64,
    };
    let items = if empty {
        Vec::new()
    } else {
        vec![
            item(
                "C1",
                None,
                "#general",
                DraftKind::Channel,
                "Proposal for accent presets: keep five, drop the free picker until the contrast check lands",
                300,
            ),
            item(
                "D1",
                None,
                "Ana Reyes",
                DraftKind::Direct,
                "Can you check the dark baselines before Friday?",
                3_600,
            ),
            item(
                "C2",
                Some("1700000000.000100"),
                "#design",
                DraftKind::Thread,
                "I think the flake is the store test under load, not the theme change.",
                DAY,
            ),
            item(
                "C3",
                None,
                "#random",
                DraftKind::Channel,
                "Draft announcement for the site launch",
                3 * DAY,
            ),
        ]
    };
    let mut s = composer(name, None);
    s.saved_drafts = Some(Drafts { items, view: true });
    s
}

/// The sidebar with Channels and Direct messages collapsed: only the unread
/// rows and the open one stay, under a closed chevron.
fn sidebar_sections_collapsed() -> Scenario {
    let mut s = sidebar_busy();
    s.name = "sidebar_sections_collapsed";
    s.collapsed = vec![
        components::SidebarSection::Channels,
        components::SidebarSection::DirectMessages,
    ];
    s
}

fn custom_section(id: u32, name: &str, convs: &[&str]) -> settings::CustomSection {
    settings::CustomSection {
        id,
        name: name.into(),
        conversations: convs.iter().map(|c| c.to_string()).collect(),
    }
}

/// The user's own sections above Channels: "Focus" holds two channels and a
/// direct message, "Clients" the private channel, "Launch" is still empty.
fn sidebar_custom_sections() -> Scenario {
    let mut s = sidebar_busy();
    s.name = "sidebar_custom_sections";
    s.sidebar_sections = vec![
        custom_section(1, "Focus", &["C2", "C4", "D1"]),
        custom_section(2, "Clients", &["C5"]),
        custom_section(3, "Launch", &[]),
    ];
    s
}

/// Organise mode: handles on every row, the section "..." buttons, Done, and
/// the "New section" field under the list.
fn sidebar_organise() -> Scenario {
    let mut s = sidebar_custom_sections();
    s.name = "sidebar_organise";
    s.sidebar_organise = true;
    s
}

/// A custom section's menu: Rename, Move up, Move down, Delete section.
fn sidebar_section_menu() -> Scenario {
    let mut s = sidebar_custom_sections();
    s.name = "sidebar_section_menu";
    s.section_menu = Some(2);
    s
}

/// The sidebar settings popover from the sidebar header.
fn sidebar_settings_popover() -> Scenario {
    let mut s = sidebar_busy();
    s.name = "sidebar_settings_popover";
    s.sidebar_settings = true;
    s
}

/// The Reminders view for a provider that keeps none: upcoming, recurring and
/// completed reminders, with the local note. The overdue one is about a message.
fn reminders_view(name: &'static str, empty: bool) -> Scenario {
    reminders_stage(name, empty, None, None)
}

fn reminders_stage(
    name: &'static str,
    empty: bool,
    edit: Option<&'static str>,
    snooze: Option<&'static str>,
) -> Scenario {
    let item = |id: &str, text: &str, due: Option<u64>, recurring: bool, completed: bool| {
        domain::Reminder {
            id: domain::ReminderId(Arc::from(id)),
            text: Arc::from(text),
            due: due.map(|d| d as i64),
            recurring,
            completed,
            fired: false,
            interval_secs: if recurring { 7 * DAY as i64 } else { 0 },
            for_user: uid("u1"),
            link: (id == "R0").then(|| {
                domain::ReminderLink::new(
                    domain::MessageId {
                        conversation: cid("C1"),
                        ts: ts(&at(T0, 1)),
                    },
                    Some(uid("u2")),
                    "Can you sanity check the contrast on the dark sidebar?",
                )
            }),
            snoozed_until: None,
        }
    };
    let items = Some(if empty {
        Vec::new()
    } else {
        vec![
            item(
                "R0",
                "Reply about the sidebar contrast",
                Some(NOW_SECS - 7_200),
                false,
                false,
            ),
            item(
                "R1",
                "Send the release notes",
                Some(NOW_SECS + 3_600),
                false,
                false,
            ),
            item(
                "R2",
                "Call the vendor back",
                Some(NOW_SECS + DAY + 5_400),
                false,
                false,
            ),
            item("R3", "Water the plants", None, false, false),
            item(
                "R4",
                "Weekly planning",
                Some(NOW_SECS + 2 * DAY),
                true,
                false,
            ),
            item("R5", "Book the venue", Some(NOW_SECS - DAY), false, true),
        ]
    });
    let mut s = composer(name, None);
    s.reminders = Some(Reminders {
        items,
        local: true,
        view: true,
        edit,
        snooze,
    });
    s
}

/// `thread_open` where the provider can follow threads and this one is followed.
fn thread_followed() -> Scenario {
    let mut s = thread_open();
    s.name = "thread_followed";
    s.followed = Some(true);
    s
}

fn profile_of(user: &str, name: &str) -> domain::Profile {
    domain::Profile {
        user: uid(user),
        display_name: Arc::from(name),
        real_name: Arc::from(name),
        title: None,
        pronouns: None,
        tz: None,
        status: None,
        email: None,
        phone: None,
        fields: Vec::new(),
        image: None,
        is_bot: false,
        deleted: false,
    }
}

/// The channel details panel beside a channel with a topic and people: the
/// header's star, stack, Huddle and details buttons show, and the panel is on
/// `tab`.
/// `recent`: the provider lists no members, so the panel falls back to the
/// people who wrote in the loaded window.
fn channel_details(name: &'static str, tab: components::DetailsTab, recent: bool) -> Scenario {
    channel_details_with(name, tab, recent, false)
}

/// [`channel_details`]; `admin`: the provider has channel admin, so About
/// shows Edit and Add people.
fn channel_details_with(
    name: &'static str,
    tab: components::DetailsTab,
    recent: bool,
    admin: bool,
) -> Scenario {
    let p = if admin {
        base().with_capabilities(domain::Capabilities {
            channel_admin: true,
            ..Default::default()
        })
    } else {
        base()
    };
    let mut c = conv("C1", "design-system");
    c.topic = Arc::from("Tokens, components and the theme format.");
    c.member_count = Some(42);
    if !recent {
        c.member_ids = Arc::from([uid("u1"), uid("u2"), uid("u3"), uid("u4"), uid("u5")]);
    }
    p.add_conversation(c);
    for (id, who) in [
        ("u2", "Aiko Tanaka"),
        ("u3", "Jonas Weber"),
        ("u4", "Priya Raman"),
        ("u5", "Lena Fischer"),
    ] {
        p.add_member(member(id, who));
    }
    p.add_messages(
        &cid("C1"),
        vec![
            from(
                "C1",
                &at(T0, 1),
                "u4",
                "Personal overrides stay tiny: one object in your theme file.",
            ),
            from(
                "C1",
                &at(T0 + 60, 1),
                "u3",
                "Added Sam and Lena to the channel for the review.",
            ),
        ],
    );
    let mut s = scenario(name, p);
    s.open = Some(cid("C1"));
    s.chrome = Some(Chrome {
        me: false,
        icon: None,
        rail: None,
    });
    s.presence = vec![(uid("u2"), true), (uid("u3"), true), (uid("u4"), false)];
    s.details = Some(tab);
    s
}

/// Members with profile pictures in the channel details list (presence dots
/// on their corners) beside a message list with the same pictures. The test
/// pictures have dark blue corners (`edge-*` in the stage's fetcher), so a
/// square photo showing outside its rounded edge stands out.
fn profile_pictures() -> Scenario {
    let s = channel_details("profile_pictures", components::DetailsTab::Members, false);
    for (id, who, pic) in [
        ("u2", "Aiko Tanaka", "edge-aiko"),
        ("u3", "Jonas Weber", "edge-jonas"),
        ("u4", "Priya Raman", "edge-priya"),
        ("u5", "Lena Fischer", "edge-wide-lena"),
    ] {
        let mut m = member(id, who);
        let url = ImageRef::Web(Arc::from(format!("https://avatars.example.test/{pic}.png")));
        m.avatar = domain::Avatar::new(Some(url.clone()), Some(url));
        s.provider.add_member(m);
    }
    s
}

/// [`people_directory`] with profile pictures on the cards (large avatars with
/// presence dots).
fn people_directory_pictures() -> Scenario {
    let s = people_directory("people_directory_pictures", "");
    for (id, n, pic) in [
        ("u2", "Aiko Tanaka", "edge-aiko"),
        ("u3", "Jonas Weber", "edge-jonas"),
        ("u4", "Priya Raman", "edge-wide-priya"),
        ("u5", "Sam Okafor", "edge-sam"),
    ] {
        let mut m = member(id, n);
        let url = ImageRef::Web(Arc::from(format!("https://avatars.example.test/{pic}.png")));
        m.avatar = domain::Avatar::new(Some(url.clone()), Some(url));
        s.provider.add_member(m);
    }
    s
}

/// The People directory beside `#general`, as on the DirectMessages board:
/// designers with titles, local times and presence, a person without either
/// and a bot (hidden by the default filters). `query` is typed in the field.
fn people_directory(name: &'static str, query: &'static str) -> Scenario {
    let mut s = general(
        name,
        vec![from(
            "C1",
            &at(T0, 1),
            "u2",
            "The new cards are in the canvas.",
        )],
    );
    let people = [
        (
            "u2",
            "Aiko Tanaka",
            Some("Product designer"),
            Some("Asia/Tokyo"),
            Some(false),
        ),
        (
            "u3",
            "Jonas Weber",
            Some("Design engineer"),
            Some("Europe/Berlin"),
            Some(true),
        ),
        (
            "u4",
            "Priya Raman",
            Some("Design lead"),
            Some("Asia/Kolkata"),
            Some(true),
        ),
        (
            "u5",
            "Sam Okafor",
            Some("Brand designer"),
            Some("Africa/Lagos"),
            Some(false),
        ),
        (
            "u6",
            "Lena Novak",
            Some("UX researcher"),
            Some("Europe/Prague"),
            None,
        ),
        (
            "u7",
            "Tomás Silva",
            Some("Motion designer"),
            Some("Europe/Lisbon"),
            Some(false),
        ),
        ("u8", "Noor Haddad", None, None, None),
    ];
    for (id, n, title, tz, active) in people {
        let mut m = member(id, n);
        m.title = title.map(Arc::from);
        m.tz = tz.map(Arc::from);
        s.provider.add_member(m);
        if let Some(a) = active {
            s.presence.push((uid(id), a));
        }
    }
    let mut bot = member("b1", "Deploy Bot");
    bot.is_bot = true;
    s.provider.add_member(bot);
    s.people = Some(PeopleStage { query });
    s
}

/// Ana Reyes' profile beside the conversation: status, local time, pause,
/// huddle, contact details and custom fields, with a "Message" button.
fn profile_panel() -> Scenario {
    let mut s = general(
        "profile_panel",
        vec![
            from("C1", &at(T0, 1), "u2", "Morning, the cache layout is in."),
            from("C1", &at(T0 + 60, 1), "u3", "Thanks, looking now."),
        ],
    );
    s.provider.add_member({
        let mut m = member("u2", "Ana Reyes");
        m.status = Some(domain::Status {
            emoji: Some(emoji("thumbsup", "\u{1f44d}")),
            text: Arc::from("Reviewing"),
            expires: None,
        });
        m
    });
    let mut dm = conv("D1", "Ana Reyes");
    dm.kind = ConversationKind::Direct;
    dm.peer = Some(uid("u2"));
    s.provider.add_conversation(dm);
    let mut p = profile_of("u2", "Ana Reyes");
    p.real_name = Arc::from("Ana Reyes Soto");
    p.pronouns = Some(Arc::from("she/her"));
    p.title = Some(Arc::from("Staff engineer"));
    p.tz = Some(Arc::from("Asia/Kolkata"));
    p.status = Some(domain::Status {
        emoji: Some(emoji("thumbsup", "\u{1f44d}")),
        text: Arc::from("Reviewing"),
        expires: None,
    });
    p.email = Some(Arc::from("ana@example.com"));
    p.phone = Some(Arc::from("+34 600 000 000"));
    p.fields = vec![
        (Arc::from("Team"), Arc::from("Platform")),
        (Arc::from("Office"), Arc::from("Barcelona")),
    ];
    s.profile = Some(ProfileStage {
        profile: p,
        dm: true,
        dnd_secs: Some(1_800),
        huddle: true,
        presence_setting: None,
        dialog: false,
        dialog_custom: false,
        dialog_preset: None,
    });
    s
}

/// The general channel with a channel admin dialog open over it.
fn channel_admin(name: &'static str, stage: AdminStage) -> Scenario {
    let mut s = general(
        name,
        vec![from(
            "C1",
            &at(T0, 1),
            "u2",
            "Morning, the cache layout is in.",
        )],
    );
    s.admin = Some(stage);
    s
}

fn browse_stage() -> AdminStage {
    let row =
        |id: &str, name: &str, private, purpose: &str, members, is_member| components::BrowseRow {
            id: ConversationId(Arc::from(id)),
            name: name.to_string().into(),
            private,
            purpose: purpose.to_string().into(),
            members: Some(members),
            is_member,
        };
    AdminStage::Browse(vec![
        row(
            "C20",
            "design-crit",
            false,
            "Show work early, get feedback on Fridays",
            14,
            false,
        ),
        row(
            "C1",
            "general",
            false,
            "Company wide news and announcements",
            42,
            true,
        ),
        row("C21", "launch-plan", true, "", 5, true),
        row(
            "C22",
            "random",
            false,
            "Anything that does not fit elsewhere",
            38,
            false,
        ),
    ])
}

fn invite_stage() -> AdminStage {
    let row = |id: &str, name: &str, handle: &str| components::InviteRow {
        id: domain::UserId(Arc::from(id)),
        name: name.to_string().into(),
        detail: handle.to_string().into(),
    };
    AdminStage::Invite {
        title: "Add people to #general",
        people: vec![
            row("u4", "Aiko Tanaka", "@aiko"),
            row("u5", "Bruno Costa", "@bruno"),
            row("u6", "Chen Wei", ""),
        ],
        picked: vec![1],
    }
}

/// The own profile with the status dialog open over the window.
fn status_dialog(name: &'static str, custom: bool) -> Scenario {
    let mut s = general(
        name,
        vec![from(
            "C1",
            &at(T0, 1),
            "u2",
            "Morning, the cache layout is in.",
        )],
    );
    let mut p = profile_of(SELF, "Me");
    p.title = Some(Arc::from("Engineer"));
    p.tz = Some(Arc::from("Europe/Madrid"));
    s.profile = Some(ProfileStage {
        profile: p,
        dm: false,
        dnd_secs: None,
        huddle: false,
        presence_setting: Some(domain::PresenceSetting::Auto),
        dialog: true,
        dialog_custom: custom,
        dialog_preset: None,
    });
    s
}

/// The status dialog with the "Focusing" preset picked.
fn status_presets() -> Scenario {
    let mut s = status_dialog("status_presets", false);
    if let Some(p) = s.profile.as_mut() {
        p.dialog_preset = Some(0);
    }
    s
}

/// A direct message with Aiko: avatar with presence, away, her local time
/// (Tokyo) and status in the header, Huddle and View profile.
fn dm_scenario(name: &'static str, messages: bool) -> Scenario {
    let p = base();
    p.add_member({
        let mut m = member("u5", "Aiko Tanaka");
        // A profile picture: the header, the intro and the message row show it
        // in place of the initials ("Me" has none and keeps them).
        let pic = ImageRef::Web(Arc::from("https://avatars.example.test/aiko.png"));
        m.avatar = domain::Avatar::new(Some(pic.clone()), Some(pic));
        m.status = Some(domain::Status {
            emoji: Some(emoji("headphones", "\u{1f3a7}")),
            text: Arc::from("Reviewing baselines until Friday"),
            expires: None,
        });
        m
    });
    let mut dm = conv("D5", "Aiko Tanaka");
    dm.kind = ConversationKind::Direct;
    dm.peer = Some(uid("u5"));
    p.add_conversation(dm);
    if messages {
        p.add_messages(
            &cid("D5"),
            vec![
                from("D5", &at(T0, 1), "u5", "Morning! The new baselines are up."),
                from("D5", &at(T0 + 60, 1), SELF, "Great, looking now."),
            ],
        );
    }
    let mut s = scenario(name, p);
    s.open = Some(cid("D5"));
    s.presence = vec![(uid("u5"), false)];
    let mut prof = profile_of("u5", "Aiko Tanaka");
    prof.tz = Some(Arc::from("Asia/Tokyo"));
    s.profiles = vec![prof];
    s.chrome = Some(Chrome {
        me: false,
        icon: None,
        rail: None,
    });
    s
}

/// The rail ends with Settings and the signed-in user's avatar (active).
fn rail_self() -> Scenario {
    let mut s = rail_and_header();
    s.name = "rail_self";
    s.presence = vec![(uid(SELF), true)];
    if let Some(c) = s.chrome.as_mut() {
        c.me = true;
    }
    s
}

/// Nine workspaces, the sixth selected, with Settings and the avatar: the list
/// scrolls between the top entries and the pinned add tile, gear and avatar.
fn rail_scroll() -> Scenario {
    let mut s = rail_and_header();
    s.name = "rail_scroll";
    s.presence = vec![(uid(SELF), true)];
    let items: Vec<_> = (0..9)
        .map(|i| {
            rail_ws(
                &format!("w{i}"),
                &format!("Workspace {}", i + 1),
                0,
                false,
                i == 5,
            )
        })
        .map(components::rail::RailItem::Workspace)
        .collect();
    if let Some(c) = s.chrome.as_mut() {
        c.me = true;
        c.rail = Some(items.into());
    }
    s
}

/// Pins, bookmarks, Later and Threads for a provider that has them: `show`
/// picks what is open, `menu` opens the actions menu on the second message.
fn collections(name: &'static str, show: CollectionsShow, menu: bool) -> Scenario {
    let msgs = vec![
        from("C1", &at(T0, 1), "u2", "Release candidate is tagged."),
        from(
            "C1",
            &at(T0 + 60, 1),
            "u3",
            "The migration runs in under a minute on the sample data.",
        ),
        from("C1", &at(T0 + 120, 1), "u4", "Thanks, merging after lunch."),
    ];
    let mut s = general(name, msgs.clone());
    s.provider.add_conversation(conv("C2", "design"));
    let mut design = from("C2", &at(T0 + 300, 1), "u2", "Spacing tokens are final.");
    design.reply_count = 4;
    let mut older = from(
        "C1",
        &at(T0 - DAY, 1),
        "u4",
        "Rollout checklist: backups, flags, comms.",
    );
    older.reply_count = 2;
    let link = |id: &str, title: &str, url: &str| domain::Bookmark {
        id: domain::BookmarkId(Arc::from(id)),
        conversation: cid("C1"),
        title: Arc::from(title),
        link: Arc::from(url),
        emoji: None,
    };
    s.collections = Some(Collections {
        pins: Some(vec![
            domain::Pin {
                message: msgs[1].id.clone(),
                by: Some(uid("u2")),
                at: (T0 + 600) as i64,
            },
            domain::Pin {
                message: older.id.clone(),
                by: Some(uid(SELF)),
                at: (T0 + 300) as i64,
            },
        ]),
        bookmarks: Some(vec![
            link("B1", "Runbook", "https://example.com/runbook"),
            link("B2", "Dashboard", "https://example.com/dash"),
            link("B3", "Oncall", "mailto:oncall@example.com"),
        ]),
        saved: Some(domain::Page {
            items: vec![
                domain::SavedItem {
                    message: msgs[0].id.clone(),
                    saved_at: (T0 + 30) as i64,
                    due: Some((T0 + 3 * DAY) as i64),
                    done: false,
                },
                domain::SavedItem {
                    message: design.id.clone(),
                    saved_at: (T0 + 400) as i64,
                    due: None,
                    done: false,
                },
                domain::SavedItem {
                    message: older.id.clone(),
                    saved_at: (T0 - 3_600) as i64,
                    due: None,
                    done: true,
                },
            ],
            next: Some(Arc::from("more")),
        }),
        inbox: Some(domain::Page {
            items: vec![
                domain::InboxThread {
                    root: design.id.clone(),
                    unread_replies: 3,
                    mentions: 1,
                    latest: ts(&at(T0 + 900, 1)),
                },
                domain::InboxThread {
                    root: older.id.clone(),
                    unread_replies: 0,
                    mentions: 0,
                    latest: ts(&at(T0 - 60, 1)),
                },
            ],
            next: None,
        }),
        held: vec![design, older],
        show,
        menu: menu.then(|| ts(&at(T0 + 60, 1))),
    });
    if show == CollectionsShow::Pins || name == "bookmarks_bar" {
        s.chrome = Some(Chrome {
            me: false,
            icon: None,
            rail: None,
        });
    }
    s
}

/// The composer's emoji button picker on the Custom tab, over many custom emoji.
fn emoji_picker_custom() -> Scenario {
    let mut s = composer(
        "emoji_picker_custom",
        Some(Draft {
            text: "Shipping on Friday ".into(),
            emoji_picker: true,
            emoji_picker_custom: true,
            ..Default::default()
        }),
    );
    s.custom_emoji_capability = true;
    for (i, code) in [
        "partyparrot",
        "partyparty",
        "shipit",
        "taco",
        "tada",
        "thumbsup",
        "rocket",
        "sparkles",
        "fire",
        "eyes",
        "wave",
        "heart",
        "coffee",
        "pizza",
        "cat",
        "dog",
        "sun",
        "moon",
        "star",
        "penguin",
    ]
    .into_iter()
    .enumerate()
    {
        s.provider.add_custom_emoji(CustomEmoji {
            shortcode: code.into(),
            image: ImageRef::Web(format!("https://example.invalid/emoji/{i}.png").into()),
        });
    }
    s
}

/// A composer with a picker open on `text`, over a workspace with a few channels.
fn composer_picker(name: &'static str, text: &str) -> Scenario {
    let mut s = composer(
        name,
        Some(Draft {
            text: text.into(),
            typed: true,
            ..Default::default()
        }),
    );
    for (id, channel) in [("C2", "design"), ("C3", "general-chat"), ("C4", "releases")] {
        s.provider.add_conversation(conv(id, channel));
    }
    // Custom emoji keep the emoji picker free of glyphs the test font lacks.
    s.custom_emoji_capability = true;
    for code in ["partyparrot", "partyparty"] {
        s.provider.add_custom_emoji(CustomEmoji {
            shortcode: code.into(),
            image: ImageRef::Web("https://example.invalid/e.png".into()),
        });
    }
    s
}

/// The `@` picker with user groups among the people: handle, name and size.
fn composer_mention_groups() -> Scenario {
    let s = composer_picker("composer_mention_groups", "Ping @de");
    for (id, handle, name, count, mine) in [
        ("S1", "design", "Design crew", 9, true),
        ("S2", "desktop", "Desktop app", 4, false),
    ] {
        s.provider.add_user_group(domain::UserGroup {
            id: id.into(),
            handle: handle.into(),
            name: name.into(),
            member_count: count,
            is_member: mine,
        });
    }
    s
}

/// Files staged on the draft: one uploading with a percentage, one ready, one failed.
fn composer_attachments() -> Scenario {
    let mut s = composer(
        "composer_attachments",
        Some(Draft {
            text: "Before and after for the dark sidebar, plus the tokens.".into(),
            ..Default::default()
        }),
    );
    s.uploads = vec![
        ("sidebar-dark.png".into(), UploadState::Progress(0.64)),
        ("parla.tokens.json".into(), UploadState::Ready),
        ("walkthrough.mov".into(), UploadState::Failed),
    ];
    s
}

fn files(name: &'static str, uploads: Vec<(String, UploadState)>) -> Scenario {
    let mut image = from("C1", &at(T0, 1), "u2", "Screenshot attached.");
    image.files = Arc::from([file(
        "F1",
        "screenshot.png",
        "image/png",
        48_000,
        FileKind::Image,
    )]);
    let mut pdf = from("C1", &at(T0 + 30, 1), "u3", "Quarterly report.");
    pdf.files = Arc::from([file(
        "F2",
        "report-q3.pdf",
        "application/pdf",
        2_400_000,
        FileKind::Pdf,
    )]);
    let mut other = from("C1", &at(T0 + 60, 1), "u4", "Raw export.");
    other.files = Arc::from([file(
        "F3",
        "export.bin",
        "application/octet-stream",
        12_345,
        FileKind::Other,
    )]);
    let mut s = general(name, vec![image, pdf, other]);
    s.uploads = uploads;
    s
}

/// `#general`'s Files tab: the files of its messages, newest first (an
/// image, a PDF, a binary export, a snippet in a thread and a clip), or its
/// empty state when no message has a file.
fn files_tab(name: &'static str, with_files: bool) -> Scenario {
    let mut s = if with_files {
        let base = files(name, Vec::new());
        let mut snippet = from("C1", &at(T0 + 90, 1), "u2", "The resolver.");
        let mut f = file(
            "F4",
            "resolve_tokens.rs",
            "text/plain",
            1_400,
            FileKind::Snippet,
        );
        f.title = Some(Arc::from("resolve_tokens.rs"));
        snippet.files = Arc::from([f]);
        let mut clip = from("C1", &at(T0 + 120, 1), "u3", "Walkthrough.");
        clip.files = Arc::from([file(
            "F5",
            "walkthrough.mp4",
            "video/mp4",
            8_200_000,
            FileKind::Video,
        )]);
        base.provider.add_messages(&cid("C1"), vec![snippet, clip]);
        base
    } else {
        general(
            name,
            vec![from("C1", &at(T0, 1), "u2", "No attachments here yet.")],
        )
    };
    s.chrome = Some(Chrome {
        me: false,
        icon: None,
        rail: None,
    });
    s.files_tab = true;
    s
}

/// Rust source of `lines` lines, for the snippet scenarios.
fn rust_source(lines: usize) -> String {
    let head = [
        "// Resolve a theme's parla object, falling back to interface tokens.",
        "pub fn resolve(appearance: Appearance, colors: &ThemeColors) -> ParlaTokens {",
        "    let d = LogoTokens::defaults(appearance);",
        "    ParlaTokens {",
        "        sidebar_background: o.sidebar_background.unwrap_or(colors.panel_background),",
        "        sidebar_text: o.sidebar_text.unwrap_or(colors.text),",
        "        presence_away: o.presence_away.unwrap_or(status.warning),",
        "        logo: LogoTokens { background: o.logo_background.unwrap_or(d.background) },",
    ];
    let mut out: Vec<String> = head.iter().map(|l| l.to_string()).collect();
    while out.len() + 2 < lines {
        out.push(format!(
            "        token_{}: o.token_{}.unwrap_or(colors.text),",
            out.len(),
            out.len()
        ));
    }
    out.push("    }".into());
    out.push("}".into());
    out.join("\n")
}

/// A snippet message in the timeline, collapsed or expanded.
fn snippet_card(name: &'static str, expanded: bool) -> Scenario {
    let mut m = from("C1", &at(T0, 1), "u2", "shared a snippet");
    let mut f = file(
        "F1",
        "resolve_tokens.rs",
        "text/plain",
        1_400,
        FileKind::Snippet,
    );
    f.preview = Some(Arc::from(rust_source(14).as_str()));
    // The provider's preview is the whole file.
    f.lines = Some(14);
    m.files = Arc::from([f]);
    let mut s = general(name, vec![m]);
    if expanded {
        s.expanded_snippets = vec!["F1"];
    }
    s
}

/// A clip open in the timeline, paused with its controls showing.
#[derive(Clone, Debug)]
pub struct StagedClip {
    /// The clip's file id.
    pub file: &'static str,
    pub position_ms: u64,
    /// Presses of the speed control (1x, 1.5x, 2x).
    pub speed_steps: usize,
    pub volume: f32,
    /// Captions turned on, loaded from this WebVTT text.
    pub captions: Option<&'static str>,
    /// The seek bar has keyboard focus (its ring shows).
    pub focus_seek: bool,
}

/// A paused video clip with its controls: seek bar, time, 1.5x speed, volume
/// and the caption showing at the position.
fn clip_controls() -> Scenario {
    let mut m = from("C1", &at(T0, 1), "u3", "Walkthrough of the new sidebar.");
    let mut f = file(
        "F7",
        "walkthrough.mp4",
        "video/mp4",
        8_200_000,
        FileKind::Video,
    );
    f.video = Some(domain::VideoInfo {
        mp4_low: Some(domain::RemoteUrl(Arc::from(
            "https://files.example/low.mp4",
        ))),
        mp4: None,
        poster: Some(domain::Rendition {
            url: domain::RemoteUrl(Arc::from("https://files.example/poster.png")),
            width: 320,
            height: 180,
        }),
        duration_ms: Some(95_000),
        vtt: Some(domain::RemoteUrl(Arc::from(
            "https://files.example/clip.vtt",
        ))),
    });
    m.files = Arc::from([f]);
    let mut s = general("clip_controls", vec![m]);
    s.clip = Some(StagedClip {
        file: "F7",
        position_ms: 32_000,
        speed_steps: 1,
        volume: 0.6,
        captions: Some(
            "WEBVTT\n\n00:30.000 --> 00:36.000\nThe sidebar keeps your sections where you left them.\n",
        ),
        focus_seek: true,
    });
    s
}

/// A long paste held back, with the snippet offer open over the composer.
fn composer_snippet_offer() -> Scenario {
    composer(
        "composer_snippet_offer",
        Some(Draft {
            snippet_paste: Some(rust_source(42)),
            ..Default::default()
        }),
    )
}

fn banner(name: &'static str, kind: Banner) -> Scenario {
    let mut s = general(name, vec![from("C1", &at(T0, 1), "u2", "Anyone there?")]);
    s.banner = Some(kind);
    s
}

/// The main window with the toast stack (the host pushes the toasts).
fn toast_stack() -> Scenario {
    general(
        "toast_stack",
        vec![
            from("C1", &at(T0, 1), "u2", "Anyone there?"),
            from(
                "C1",
                &at(T0, 2),
                "u2",
                "Here is the link to the design review.",
            ),
        ],
    )
}

fn empty_conversation() -> Scenario {
    general("empty_conversation", Vec::new())
}

/// The first history page is still in flight: the skeleton.
fn timeline_loading() -> Scenario {
    let mut s = general("timeline_loading", Vec::new());
    s.history = LoadStage::Loading;
    s
}

/// The thread's replies are still in flight.
fn thread_loading() -> Scenario {
    let mut s = thread_open();
    s.name = "thread_loading";
    s.replies = LoadStage::Loading;
    s
}

/// The thread's replies failed to load.
fn thread_load_error() -> Scenario {
    let mut s = thread_open();
    s.name = "thread_load_error";
    s.replies = LoadStage::Failed;
    s
}

/// The first history page failed: the message and Retry.
fn timeline_load_error() -> Scenario {
    let mut s = general("timeline_load_error", Vec::new());
    s.history = LoadStage::Failed;
    s
}

/// A workspace with `#general` open and 26 conversations in "Direct messages",
/// four of them unread or open beyond the recent ones. Collapsed, the section
/// ends in "Show more"; expanded, in "Show fewer".
fn dm_section(name: &'static str, expanded: bool) -> Scenario {
    let p = base();
    p.add_conversation(conv("C1", "general"));
    let people = ["Ana Reyes", "Ben Okafor", "Cy Lindqvist"];
    for i in 0..26usize {
        let mut d = conv(&format!("D{i:02}"), people[i % 3]);
        d.kind = ConversationKind::Direct;
        d.peer = Some(uid(["u2", "u3", "u4"][i % 3]));
        // The higher the number, the newer the last message.
        d.latest = Some(MessageTs(Arc::from(at(T0 + 60 * i as u64, 1))));
        // An old unread conversation stays listed when collapsed.
        d.unread = i == 1;
        p.add_conversation(d);
    }
    // The oldest conversation is open: the list scrolls to it, which keeps
    // it listed when collapsed and brings the last row into view.
    p.add_message(from("D00", &at(T0, 2), "u2", "See you on Monday."));
    let mut s = scenario(name, p);
    s.open = Some(cid("D00"));
    s.dms_expanded = expanded;
    s
}

fn long_names() -> Scenario {
    let p = base();
    let long = "a-very-long-channel-name-that-must-be-cut-with-an-ellipsis-at-the-column-edge";
    p.add_conversation(conv("C1", long));
    let mut dm = conv("D1", "Bartholomew Featherstonehaugh-Cholmondeley the Third");
    dm.kind = ConversationKind::Direct;
    dm.peer = Some(uid("u2"));
    dm.unread = true;
    dm.mentions = 12;
    p.add_conversation(dm);
    p.add_member(member(
        "u5",
        "Bartholomew Featherstonehaugh-Cholmondeley the Third",
    ));
    p.add_message(from(
        "C1",
        &at(T0, 1),
        "u5",
        "A message from a person with a very long display name, with an unbroken token: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ));
    let mut s = scenario("long_names", p);
    s.open = Some(cid("C1"));
    s
}

/// Authors, a replier and a bot missing from the directory: the session looks
/// them up; one cannot be resolved and reads as the neutral placeholder.
fn unresolved_names() -> Scenario {
    let mut s = scenario(
        "unresolved_names",
        domain::test_support::fixture::unresolved_names_workspace(),
    );
    s.open = Some(cid("C1"));
    s
}

/// The rail with the workspace icon, and a header whose topic links a channel
/// and a person: `#random` and `@Ana` in the topic read as links.
fn rail_and_header() -> Scenario {
    let p = base();
    let mut c = conv("C1", "general");
    c.topic = Arc::from("Release talk, see #random and ping @Ana for the plan");
    c.member_count = Some(42);
    p.add_conversation(c);
    p.add_conversation(conv("C2", "random"));
    p.add_messages(
        &cid("C1"),
        vec![from(
            "C1",
            &at(T0, 1),
            "u2",
            "The topic above links to the other channel.",
        )],
    );
    let mut s = scenario("rail_and_header", p);
    s.open = Some(cid("C1"));
    s.chrome = Some(Chrome {
        me: false,
        icon: Some(ImageRef::Web(
            "https://example.invalid/team-icon.png".into(),
        )),
        rail: None,
    });
    s
}

fn rail_ws(
    id: &str,
    name: &str,
    mentions: u32,
    unread: bool,
    active: bool,
) -> components::rail::RailWorkspace {
    components::rail::RailWorkspace {
        key: settings::WorkspaceKey::new("fake", id),
        name: name.to_string().into(),
        icon: None,
        initials: components::badge::initials(name).into(),
        mentions,
        unread,
        active,
        expired: false,
        lean: false,
        paused: false,
        accent: None,
    }
}

/// Three workspaces on the rail: the active one with an accent, one with
/// mentions, one with an expired session that went lean.
fn rail_multi() -> Scenario {
    let mut s = rail_and_header();
    s.name = "rail_multi";
    let mut a = rail_ws("w1", "Example Workspace", 0, false, true);
    a.accent = components::rail::parse_accent("#3b82f6");
    let b = rail_ws("w2", "Globex Labs", 4, true, false);
    let mut c = rail_ws("w3", "Initech", 0, true, false);
    c.expired = true;
    c.lean = true;
    s.chrome = Some(Chrome {
        me: false,
        icon: None,
        rail: Some(
            [a, b, c]
                .into_iter()
                .map(components::rail::RailItem::Workspace)
                .collect::<Vec<_>>()
                .into(),
        ),
    });
    s
}

fn tag(id: &str, name: &str, accent: Option<&str>) -> components::WorkspaceTag {
    components::WorkspaceTag {
        key: settings::WorkspaceKey::new("fake", id),
        name: name.to_string().into(),
        accent: accent.and_then(components::rail::parse_accent),
    }
}

fn unread_item(
    ws: components::WorkspaceTag,
    label: &str,
    unread: bool,
    mentions: u32,
) -> components::UnreadItem {
    components::UnreadItem {
        workspace: ws,
        conversation: cid(label),
        label: label.to_string().into(),
        unread,
        mentions,
        latest: Some(ts("1700000100.000001")),
        last_read: None,
    }
}

/// "All unreads" with the newest unread messages under each held row.
fn all_unreads_previews() -> Scenario {
    let mut s = aggregate("all_unreads_previews", components::AggregateKind::Unreads);
    let line = |t: &str, who: &str, time: &str, text: &str| components::PreviewLine {
        id: mid("C9", t),
        author: who.to_string().into(),
        avatar: None,
        time: time.to_string().into(),
        text: text.to_string().into(),
    };
    if let Some(a) = s.aggregate.as_mut() {
        let (first, second) = (
            a.unreads[0].conversation.clone(),
            a.unreads[1].conversation.clone(),
        );
        a.previews.push((
            first,
            vec![
                line(
                    "1700000050.000001",
                    "Ana Reyes",
                    "10:02",
                    "Idle CPU is back under budget after the cursor blink fix.",
                ),
                line(
                    "1700000080.000001",
                    "Ben Okafor",
                    "10:14",
                    "The release candidate is building; the signed image lands in #launch.",
                ),
                line(
                    "1700000100.000001",
                    "Cy Lindqvist",
                    "10:20",
                    "@Me can you own the scroll pass before Friday?",
                ),
            ],
        ));
        a.previews.push((
            second,
            vec![line(
                "1700000090.000001",
                "Ana Reyes",
                "10:33",
                "Coffee after the design crit? I want to show you the accent picker.",
            )],
        ));
    }
    s
}

/// The three-workspace rail with a cross-workspace view of `kind` in the centre.
fn aggregate(name: &'static str, kind: components::AggregateKind) -> Scenario {
    let mut s = rail_multi();
    s.name = name;
    let (a, b, c) = (
        tag("w1", "Example Workspace", Some("#3b82f6")),
        tag("w2", "Globex Labs", None),
        tag("w3", "Initech", None),
    );
    let line = |t: &str, who: &str, text: &str| components::MentionLine {
        id: mid("C9", t),
        author: who.to_string().into(),
        text: text.to_string().into(),
    };
    let sched = |ws: &components::WorkspaceTag,
                 id: &str,
                 to: &str,
                 text: &str,
                 at: u64,
                 st: Option<domain::LocalScheduleState>| {
        components::ScheduledItem {
            workspace: ws.clone(),
            id: domain::ScheduleId(Arc::from(id)),
            conversation: to.to_string().into(),
            text: text.to_string().into(),
            post_at: at as i64,
            local: st.is_some(),
            state: st,
        }
    };
    s.aggregate = Some(Aggregate {
        kind,
        unreads: vec![
            unread_item(b.clone(), "#launch", true, 3),
            unread_item(a.clone(), "#general", true, 0),
            unread_item(b.clone(), "#support", true, 0),
            unread_item(c.clone(), "#random", true, 0),
        ],
        mentions: vec![
            components::MentionItem {
                row: unread_item(a.clone(), "#general", true, 2),
                messages: vec![
                    line(
                        "1700000090.000001",
                        "Ana Reyes",
                        "@Me can you review the release notes?",
                    ),
                    line("1700000050.000001", "Ben Okafor", "Thanks @Me, merged."),
                ],
            },
            components::MentionItem {
                row: unread_item(c.clone(), "#ops", true, 1),
                messages: vec![],
            },
        ],
        scheduled: vec![
            sched(
                &a,
                "S1",
                "#general",
                "Standup notes go out first thing",
                NOW_SECS + 10 * 3_600,
                None,
            ),
            sched(
                &b,
                "L1",
                "#launch",
                "Design review slides attached",
                NOW_SECS + 10 * 3_600 + 1_800,
                Some(domain::LocalScheduleState::Pending),
            ),
            sched(
                &c,
                "S2",
                "#random",
                "Reminder: freeze starts Monday",
                NOW_SECS + 2 * DAY,
                None,
            ),
        ],
        confirm: false,
        previews: Vec::new(),
    });
    s
}

/// Notification controls: levels beside the sidebar rows, the paused strip,
/// the paused mark on the rail and the level on the header.
fn notify_controls() -> Scenario {
    let mut s = rail_and_header();
    s.name = "notify_controls";
    let p = &s.provider;
    p.add_conversation(conv("C3", "design"));
    p.add_conversation(conv("C4", "announcements"));
    let mut a = rail_ws("w1", "Example Workspace", 0, false, true);
    a.paused = true;
    let b = rail_ws("w2", "Globex Labs", 4, true, false);
    s.chrome = Some(Chrome {
        me: false,
        icon: None,
        rail: Some(
            [a, b]
                .into_iter()
                .map(components::rail::RailItem::Workspace)
                .collect::<Vec<_>>()
                .into(),
        ),
    });
    let prefs = |level, muted| domain::ConversationPrefs { level, muted };
    s.notify = Some(NotifyStage {
        levels: vec![
            (cid("C1"), prefs(domain::NotifyLevel::Mentions, false)),
            (cid("C2"), prefs(domain::NotifyLevel::Default, true)),
            (cid("C3"), prefs(domain::NotifyLevel::All, false)),
            (cid("C4"), prefs(domain::NotifyLevel::Nothing, false)),
        ],
        paused_for: Some(30 * 60),
        ..Default::default()
    });
    s
}

/// The header bell's menu with every extra set: thread replies off, sound
/// off and a one-hour mute running (half an hour left).
fn notify_menu_extras() -> Scenario {
    let mut s = notify_controls();
    s.name = "notify_menu_extras";
    if let Some(n) = s.notify.as_mut() {
        n.extras = vec![(
            cid("C1"),
            domain::NotifyExtras {
                threads_off: true,
                sound_off: true,
                mute_until: Some(NOW_SECS as i64 + 30 * 60),
            },
        )];
    }
    s
}

/// The sidebar with one channel muted for an hour (the mute mark shows).
fn sidebar_timed_mute() -> Scenario {
    let mut s = sidebar_busy();
    s.name = "sidebar_timed_mute";
    s.notify = Some(NotifyStage {
        extras: vec![(
            cid("C2"),
            domain::NotifyExtras {
                mute_until: Some(NOW_SECS as i64 + 30 * 60),
                ..Default::default()
            },
        )],
        ..Default::default()
    });
    s
}

/// Two workspaces of one organisation grouped, then a standalone one with an
/// unread dot and a large mention count.
fn rail_org() -> Scenario {
    let mut s = rail_and_header();
    s.name = "rail_org";
    let a = rail_ws("w1", "Example Workspace", 0, false, true);
    let b = rail_ws("w2", "Example Support", 2, true, false);
    let c = rail_ws("w3", "Globex Labs", 120, true, false);
    let d = rail_ws("w4", "Initech", 0, true, false);
    s.chrome = Some(Chrome {
        me: false,
        icon: None,
        rail: Some(
            vec![
                components::rail::RailItem::Org {
                    name: "Example Org".into(),
                    children: vec![a, b],
                },
                components::rail::RailItem::Workspace(c),
                components::rail::RailItem::Workspace(d),
            ]
            .into(),
        ),
    });
    s
}

/// The email-code sign-in staged at `stage`.
fn code_stage(name: &'static str, stage: SignInStage) -> Scenario {
    let mut s = scenario(name, FakeProvider::new("w0", uid(SELF)));
    s.sign_in = true;
    s.sign_in_stage = Some(stage);
    s
}

/// The email-code sign-in states.
fn code_stages() -> Vec<Scenario> {
    vec![
        code_stage("sign_in_email", SignInStage::Email),
        code_stage("sign_in_email_invalid", SignInStage::EmailInvalid),
        code_stage("sign_in_email_sending", SignInStage::EmailSending),
        code_stage(
            "sign_in_email_needs_browser",
            SignInStage::EmailNeedsBrowser,
        ),
        code_stage("sign_in_email_rate_limited", SignInStage::EmailRateLimited),
        code_stage("sign_in_email_offline", SignInStage::EmailOffline),
        code_stage("sign_in_code", SignInStage::Code),
        code_stage("sign_in_code_partial", SignInStage::CodePartial),
        code_stage("sign_in_code_verifying", SignInStage::CodeVerifying),
        code_stage("sign_in_code_wrong", SignInStage::CodeWrong),
        code_stage("sign_in_code_expired", SignInStage::CodeExpired),
        code_stage("sign_in_code_too_many", SignInStage::CodeTooMany),
        code_stage("sign_in_code_rate_limited", SignInStage::CodeRateLimited),
        code_stage("sign_in_code_resend_wait", SignInStage::CodeResendWait),
        code_stage("sign_in_code_resent", SignInStage::CodeResent),
    ]
}

/// Every scenario, in a stable order; the names are the baseline names.
pub fn all() -> Vec<Scenario> {
    vec![
        sign_in(),
        sign_in_form(),
        sign_in_error(),
        sign_in_long(),
        sign_in_form_long(),
        workspace_picker(),
        sidebar_busy(),
        timeline_rich(),
        timeline_unread_divider(),
        timeline_markup(),
        timeline_bot_alert(),
        timeline_states(),
        timeline_offline(),
        composer_session_expired(),
        timeline_selected(),
        reactions(),
        thread_open(),
        popout_thread(),
        composer_join(),
        composer_typing(),
        composer(
            "composer_multiline",
            Some(Draft {
                text: "First line\nSecond line\nThird line".into(),
                ..Default::default()
            }),
        ),
        composer(
            "composer_editing",
            Some(Draft {
                text: "Anything to add before we ship? (fixing a typo)".into(),
                editing: true,
                ..Default::default()
            }),
        ),
        composer(
            "composer_selection",
            Some(Draft {
                text: "Shipping on Friday, unless the review finds something".into(),
                select: Some(12..18),
                ..Default::default()
            }),
        ),
        composer(
            "composer_composing",
            Some(Draft {
                text: "Voy a cenar, ".into(),
                composing: Some("ñand".into()),
                ..Default::default()
            }),
        ),
        composer_attachments(),
        composer(
            "composer_slash_menu",
            Some(Draft {
                text: "/".into(),
                typed: true,
                ..Default::default()
            }),
        ),
        composer(
            "composer_link_prompt",
            Some(Draft {
                text: "Notes are in the release docs".into(),
                select: Some(21..29),
                link_prompt: true,
                ..Default::default()
            }),
        ),
        composer(
            "composer_emoji_button_picker",
            Some(Draft {
                text: "Shipping on Friday ".into(),
                emoji_picker: true,
                ..Default::default()
            }),
        ),
        composer_send_later("composer_send_later_menu", SendLaterStage::Menu, true),
        composer_send_later(
            "composer_send_later_custom",
            SendLaterStage::CustomPast,
            false,
        ),
        scheduled_view("scheduled_view", false, None),
        scheduled_view("scheduled_long", true, None),
        scheduled_view("scheduled_edit", false, Some("S1")),
        collections("message_actions_menu", CollectionsShow::Nothing, true),
        collections("pins_popover", CollectionsShow::Pins, false),
        collections("bookmarks_bar", CollectionsShow::Nothing, false),
        collections("later_view", CollectionsShow::Later, false),
        collections("threads_view", CollectionsShow::Threads, false),
        reminders_view("reminders_view", false),
        drafts_view("drafts_view", false),
        drafts_view("drafts_empty", true),
        sidebar_sections_collapsed(),
        sidebar_custom_sections(),
        sidebar_organise(),
        sidebar_section_menu(),
        sidebar_settings_popover(),
        reminders_view("reminders_empty", true),
        reminders_stage("reminder_edit", false, Some("R0"), None),
        reminders_stage("reminder_snooze_menu", false, None, Some("R0")),
        thread_followed(),
        profile_panel(),
        people_directory("people_directory", ""),
        people_directory("people_directory_empty_search", "zebra"),
        channel_details(
            "channel_details_about",
            components::DetailsTab::About,
            false,
        ),
        channel_details(
            "channel_details_members",
            components::DetailsTab::Members,
            false,
        ),
        channel_details(
            "channel_details_recent",
            components::DetailsTab::Members,
            true,
        ),
        profile_pictures(),
        people_directory_pictures(),
        presence_dots(),
        huddle_awareness("huddle_awareness", false),
        huddle_awareness("huddle_self_in", true),
        call_bar("call_bar_live", true),
        call_bar("call_bar_connecting", false),
        call_bar_native("call_bar_people", false),
        call_bar_native("call_bar_devices", true),
        huddle_start(),
        channel_admin(
            "channel_create",
            AdminStage::Create {
                name: "Launch Plan",
                private: false,
                error: None,
            },
        ),
        channel_admin(
            "channel_create_error",
            AdminStage::Create {
                name: "general",
                private: true,
                error: Some("That name is already taken by a channel, a person or a user group."),
            },
        ),
        channel_admin("channel_browser", browse_stage()),
        channel_admin(
            "channel_edit",
            AdminStage::Edit {
                name: "general",
                topic: "Weekly notes and launch dates",
                purpose: "Company wide news and announcements",
            },
        ),
        channel_admin("channel_invite", invite_stage()),
        channel_admin(
            "channel_leave_confirm",
            AdminStage::Confirm {
                title: "Leave #general?",
                body: "You can join again from Browse channels.",
                ok: "Leave",
                danger: false,
            },
        ),
        channel_admin(
            "channel_archive_confirm",
            AdminStage::Confirm {
                title: "Archive #launch-plan?",
                body: "Nobody can post in it any more. Its messages stay readable and searchable.",
                ok: "Archive",
                danger: true,
            },
        ),
        channel_admin("channel_browser_loading", AdminStage::BrowseLoading),
        channel_admin("channel_browser_empty", AdminStage::BrowseEmpty),
        channel_admin(
            "channel_browser_error",
            AdminStage::BrowseError("The channel list could not be loaded."),
        ),
        channel_details_with(
            "channel_details_about_admin",
            components::DetailsTab::About,
            false,
            true,
        ),
        status_dialog("status_dialog", false),
        status_dialog("status_dialog_custom", true),
        status_presets(),
        dm_scenario("dm_header", true),
        dm_scenario("dm_intro", false),
        aggregate("all_unreads", components::AggregateKind::Unreads),
        aggregate("all_mentions", components::AggregateKind::Mentions),
        aggregate("all_scheduled", components::AggregateKind::Scheduled),
        all_unreads_previews(),
        {
            let mut s = aggregate("all_unreads_confirm", components::AggregateKind::Unreads);
            if let Some(a) = s.aggregate.as_mut() {
                a.confirm = true;
            }
            s
        },
        composer_picker("composer_mention_picker", "Thanks @an"),
        composer_mention_groups(),
        composer_picker("composer_channel_picker", "Moving this to #ge"),
        composer_picker("composer_emoji_picker", "Shipped :partypar"),
        emoji_picker_custom(),
        snippet_card("snippet_card", false),
        snippet_card("snippet_card_expanded", true),
        clip_controls(),
        composer_snippet_offer(),
        files("files", Vec::new()),
        files_tab("files_tab", true),
        files_tab("files_tab_empty", false),
        files(
            "files_upload_progress",
            vec![("design-review.mov".into(), UploadState::Progress(0.4))],
        ),
        files(
            "files_upload_failed",
            vec![("notes.txt".into(), UploadState::Failed)],
        ),
        banner("banner_session_expired", Banner::SessionExpired),
        banner("banner_config_error", Banner::ConfigError),
        banner("banner_incoming_huddle", Banner::IncomingHuddle),
        toast_stack(),
        empty_conversation(),
        timeline_loading(),
        timeline_load_error(),
        thread_loading(),
        thread_load_error(),
        dm_section("dm_section_collapsed", false),
        dm_section("dm_section_expanded", true),
        long_names(),
        unresolved_names(),
        rail_and_header(),
        rail_self(),
        rail_scroll(),
        rail_multi(),
        rail_org(),
        notify_controls(),
        notify_menu_extras(),
        sidebar_timed_mute(),
        sidebar_starred(),
        search_results(),
        search_offline(),
        search_preview(),
    ]
    .into_iter()
    .chain(code_stages())
    .collect()
}

pub fn by_name(name: &str) -> Option<Scenario> {
    all().into_iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_unique_and_snake_case() {
        let all = all();
        assert!(all.len() >= 14, "expected the full library");
        let mut seen = HashSet::new();
        for s in &all {
            assert!(seen.insert(s.name), "duplicate scenario {}", s.name);
            assert!(
                !s.name.is_empty()
                    && s.name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    && !s.name.starts_with('_')
                    && !s.name.ends_with('_'),
                "{} is not snake_case",
                s.name
            );
        }
        for required in ["sign_in", "sidebar_busy", "long_names", "thread_open"] {
            assert!(seen.contains(required), "missing {required}");
        }
    }

    #[test]
    fn by_name_round_trips() {
        for s in all() {
            let again = by_name(s.name).unwrap_or_else(|| panic!("{} not found", s.name));
            assert_eq!(again.name, s.name);
        }
        assert!(by_name("no_such_scenario").is_none());
    }

    #[test]
    fn scenarios_are_self_consistent() {
        for s in all() {
            if let Some(open) = &s.open {
                assert!(!open.0.is_empty(), "{}: empty open id", s.name);
            }
            if let Some(root) = &s.thread {
                assert!(
                    s.open.is_some(),
                    "{}: thread needs an open conversation",
                    s.name
                );
                assert!(!root.0.is_empty());
            }
        }
        assert!(by_name("sign_in").unwrap().sign_in);
        assert!(by_name("thread_open").unwrap().thread.is_some());
        assert!(by_name("composer_editing").unwrap().draft.unwrap().editing);
        assert!(by_name("banner_config_error").unwrap().banner == Some(Banner::ConfigError));
    }

    #[test]
    fn no_fixture_string_names_a_provider() {
        let brand = ["sl", "ack"].concat();
        let src = include_str!("scenario.rs");
        // The source (all fixtures live in it) never names a provider.
        let fixtures = src.split("#[cfg(test)]").next().unwrap().to_lowercase();
        assert!(!fixtures.contains(&brand));
        for s in all() {
            assert!(!format!("{:?}", s.name).to_lowercase().contains(&brand));
        }
    }
}
