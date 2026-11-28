//! A static session over a [`Scenario`]: implements every component source
//! trait from a `WorkspaceState` built once through the real reducer, so
//! components render exactly what the app would show for the same data.

use std::collections::HashSet;

use components::{
    ComposerSource, MemberIndex, SidebarRow, SidebarSource, ThreadSource, TimelineSource,
    build_rows_sections,
};
use std::sync::Arc;

use domain::{
    Block, Capabilities, ChatProvider as _, ContentParser, Conversation, ConversationId,
    CustomEmoji, Effect, EmojiRef, FileRef, Inline, Input, Load, Loaded, Member, Message,
    MessageId, MessageWindow, PlainParser, RichText, Style, UserId, WorkspaceState, reduce,
};
use futures::executor::block_on;
use gpui::{App, SharedString};
use settings::SidebarSettings;

use crate::scenario::{LoadStage, Scenario, UploadState};

/// Messages the scenario state loads per conversation.
const PAGE: u32 = 200;

/// Pre-parsed rich text a scenario attaches to a message (as its `ext`), the
/// neutral stand-in for a provider's markup parser.
pub struct RichExt(pub RichText);

/// Shows a message's [`RichExt`] when it has one, its text otherwise. A topic
/// reads `#channel` and `@Person` words as links to the scenario's own
/// conversations and members (the neutral stand-in for a provider's markup).
struct ScenarioParser {
    words: Vec<(String, Inline)>,
}

impl ScenarioParser {
    fn new(state: &WorkspaceState) -> Self {
        let mut words = Vec::new();
        for c in state.conversations.values() {
            words.push((
                format!("#{}", c.name),
                Inline::Conversation {
                    id: c.id.clone(),
                    label: Some(c.name.to_string().into()),
                },
            ));
        }
        for m in state.members.values() {
            if let Some(first) = m.display_name.split_whitespace().next() {
                words.push((
                    format!("@{first}"),
                    Inline::User {
                        id: m.id.clone(),
                        label: None,
                    },
                ));
            }
        }
        Self { words }
    }
}

impl ContentParser for ScenarioParser {
    fn parse_text(&self, text: &str) -> RichText {
        let mut items = Vec::new();
        let mut run = String::new();
        let flush = |run: &mut String, items: &mut Vec<Inline>| {
            if !run.is_empty() {
                items.push(Inline::Text {
                    text: std::mem::take(run).into(),
                    style: Style::default(),
                });
            }
        };
        for (i, word) in text.split(' ').enumerate() {
            if i > 0 {
                run.push(' ');
            }
            match self.words.iter().find(|(w, _)| w == word) {
                Some((_, inline)) => {
                    flush(&mut run, &mut items);
                    items.push(inline.clone());
                }
                None => run.push_str(word),
            }
        }
        flush(&mut run, &mut items);
        RichText {
            blocks: Arc::from(vec![Block::Paragraph(items)]),
        }
    }

    fn parse(&self, m: &Message) -> RichText {
        match m.ext.get::<RichExt>() {
            Some(r) => r.0.clone(),
            None => PlainParser.parse(m),
        }
    }
}

/// Answers `Load::Members` effects from the provider until none is left, as
/// the app's session does, so unknown authors resolve before the first paint.
fn settle_lookups(state: &mut WorkspaceState, p: &dyn domain::ChatProvider, mut fx: Vec<Effect>) {
    while let Some(ids) = fx.iter().find_map(|e| match e {
        Effect::Load(Load::Members(ids)) => Some(ids.clone()),
        _ => None,
    }) {
        let r = match block_on(p.members(&ids)) {
            Ok(found) => Ok(Loaded::Members { asked: ids, found }),
            Err(e) => Err((Load::Members(ids), e)),
        };
        fx = reduce(state, Input::Loaded(r));
    }
}

/// The data a scenario shows, behind the component source traits.
pub struct ScenarioSession {
    state: WorkspaceState,
    caps: Capabilities,
    drafts: HashSet<ConversationId>,
    selected: Option<ConversationId>,
    dms_expanded: bool,
    uploads: Vec<(u64, SharedString, UploadState)>,
    scheduled: Option<crate::scenario::Scheduled>,
    reminders: Option<crate::scenario::Reminders>,
    saved_drafts: Option<crate::scenario::Drafts>,
    collapsed: std::collections::BTreeSet<components::SidebarSection>,
    sidebar_settings: SidebarSettings,
    collections: Option<crate::scenario::Collections>,
    search: components::SearchResults,
    remote_search: bool,
    own_presence: Option<domain::PresenceSetting>,
    can_start_huddle: bool,
    in_call: bool,
    history_failed: bool,
    replies_failed: bool,
    followed: Option<bool>,
    /// The sidebar offers "People" (the directory scenarios).
    people: bool,
}

impl ScenarioSession {
    /// The zone schedule times show in: the settings override, else the profile
    /// zone, else the installed UI zone (UTC in baselines), like a real session.
    fn schedule_zone(&self, cx: &App) -> jiff::tz::TimeZone {
        let over = cx
            .try_global::<settings::SettingsStore>()
            .and_then(|s| s.current().send_later.timezone.clone());
        domain::state_schedule_zone(&self.state, over.as_deref(), components::time_zone(cx))
    }

    /// Builds the state: bootstrap, the newest page of the open conversation
    /// and, when the scenario opens a thread, its replies.
    pub fn new(s: &Scenario) -> Self {
        let p = &s.provider;
        let mut state = WorkspaceState::new(p.workspace().clone(), PAGE as usize);
        state.caps.member_lookup = p.capabilities().member_lookup;
        state.caps.channel_admin = p.capabilities().channel_admin;
        state.caps.custom_emoji = s.custom_emoji_capability;
        let boot = block_on(p.bootstrap()).expect("fake bootstrap");
        let fx = reduce(&mut state, Input::Bootstrapped(boot));
        settle_lookups(&mut state, p.as_ref(), fx);
        let custom = block_on(p.custom_emoji()).expect("fake custom emoji");
        reduce(&mut state, Input::CustomEmojiLoaded(custom));
        let groups = block_on(p.user_groups()).expect("fake user groups");
        reduce(&mut state, Input::UserGroupsLoaded(groups));
        let mut drafts = HashSet::new();
        if let Some(open) = s.open.as_ref().filter(|_| s.history == LoadStage::Loaded) {
            let page = block_on(p.history(open, None, PAGE)).expect("fake history");
            let fx = reduce(
                &mut state,
                Input::HistoryLoaded {
                    conversation: open.clone(),
                    page,
                },
            );
            settle_lookups(&mut state, p.as_ref(), fx);
            if s.draft.is_some() {
                drafts.insert(open.clone());
            }
        }
        if let (Some(open), Some(root)) = (&s.open, &s.thread) {
            let root = MessageId {
                conversation: open.clone(),
                ts: root.clone(),
            };
            reduce(&mut state, Input::OpenThread(root.clone()));
            if s.replies == LoadStage::Loaded {
                let page = block_on(p.replies(open, &root.ts, None, PAGE)).expect("fake replies");
                let fx = reduce(&mut state, Input::RepliesLoaded { root, page });
                settle_lookups(&mut state, p.as_ref(), fx);
            }
        }
        if let (Some(open), Some(target)) = (&s.open, &s.jump) {
            // Detach the window around the jump target, as a jump does.
            let messages = state
                .windows
                .get(open)
                .map(|w| w.messages().to_vec())
                .unwrap_or_default();
            reduce(
                &mut state,
                Input::AroundLoaded {
                    target: MessageId {
                        conversation: open.clone(),
                        ts: target.clone(),
                    },
                    page: domain::AroundPage {
                        messages,
                        has_more_before: true,
                        has_more_after: true,
                    },
                },
            );
        }
        state.presence = s.presence.iter().cloned().collect();
        if let Some(open) = s.open.as_ref().filter(|_| !s.typing.is_empty()) {
            reduce(
                &mut state,
                Input::Clock {
                    unix: 0,
                    mono: std::time::Instant::now(),
                },
            );
            for u in &s.typing {
                reduce(
                    &mut state,
                    Input::Provider(domain::ProviderEvent::Typing {
                        conversation: open.clone(),
                        user: u.clone(),
                        thread: None,
                    }),
                );
            }
        }
        if s.people.is_some() {
            state.caps.presence_watch = true;
        }
        for pf in &s.profiles {
            state.profiles.insert(pf.user.clone(), pf.clone());
        }
        if s.offline {
            state.connection = domain::Connection::Offline;
        }
        if s.expired {
            state.session = domain::Session::Expired;
        }
        let mut own_presence = None;
        if let Some(pf) = &s.profile {
            let u = pf.profile.user.clone();
            state.profiles.insert(u.clone(), pf.profile.clone());
            state.open_profile = Some(u.clone());
            if let Some(secs) = pf.dnd_secs {
                state.dnd.insert(
                    u.clone(),
                    domain::DndStatus {
                        snooze_until: Some(crate::scenario::NOW_SECS as i64 + secs),
                        ..Default::default()
                    },
                );
            }
            if pf.huddle {
                state.in_huddle.insert(u);
            }
            own_presence = pf.presence_setting;
        }
        state.huddles = s
            .huddles
            .iter()
            .map(|h| (h.conversation.clone(), h.clone()))
            .collect();
        state.starred = s.starred.iter().cloned().collect();
        if let Some(n) = &s.notify {
            state.local_notify_prefs = n.levels.iter().cloned().collect();
            state.local_notify_extras = n.extras.iter().cloned().collect();
            if let (Some(secs), Some(me)) = (n.paused_for, state.self_user.clone()) {
                state.dnd.insert(
                    me,
                    domain::DndStatus {
                        snooze_until: Some(crate::scenario::NOW_SECS as i64 + secs),
                        ..Default::default()
                    },
                );
            }
        }
        Self {
            state,
            caps: Capabilities {
                threads: true,
                edit: true,
                delete: true,
                reminders: true,
                dnd: true,
                status: true,
                presence: true,
                scheduled_send: s.scheduled.is_some(),
                custom_emoji: s.custom_emoji_capability,
                pins: s.collections.is_some(),
                bookmarks: s.collections.is_some(),
                saved: s.collections.is_some(),
                thread_inbox: s.collections.is_some(),
                huddles: !s.huddles.is_empty(),
                thread_follow: s.followed.is_some(),
                ..Capabilities::default()
            },
            drafts,
            selected: s.open.clone(),
            dms_expanded: s.dms_expanded,
            uploads: s
                .uploads
                .iter()
                .enumerate()
                .map(|(i, (name, st))| (i as u64, SharedString::from(name.clone()), *st))
                .collect(),
            scheduled: s.scheduled.clone(),
            reminders: s.reminders.clone(),
            saved_drafts: s.saved_drafts.clone(),
            collapsed: s.collapsed.iter().copied().collect(),
            sidebar_settings: SidebarSettings {
                sections: s.sidebar_sections.clone(),
                ..SidebarSettings::default()
            },
            collections: s.collections.clone(),
            search: s
                .search
                .as_ref()
                .map(|x| x.results.clone())
                .unwrap_or_default(),
            remote_search: s.search.as_ref().is_some_and(|x| x.remote),
            own_presence,
            can_start_huddle: s.start_huddle,
            in_call: s.call.is_some(),
            history_failed: s.history == LoadStage::Failed,
            replies_failed: s.replies == LoadStage::Failed,
            followed: s.followed,
            people: s.people.is_some(),
        }
    }

    pub fn state(&self) -> &WorkspaceState {
        &self.state
    }
}

impl components::ScheduledSource for ScenarioSession {
    fn scheduled(&self) -> &[domain::ScheduledMessage] {
        self.scheduled.as_ref().map_or(&[], |s| &s.server)
    }
    fn local(&self) -> &[domain::LocalSchedule] {
        self.scheduled.as_ref().map_or(&[], |s| &s.local)
    }
    fn reschedule(&mut self, _: domain::ScheduledMessage, _: String, _: i64, _: &mut App) {}
    fn delete(&mut self, _: domain::ScheduleId, _: &mut App) {}
    fn send_now(&mut self, _: domain::ScheduleId, _: &mut App) {}
    fn conversation_label(&self, c: &ConversationId, _: &App) -> String {
        self.state
            .conversations
            .get(c)
            .map_or_else(|| c.0.to_string(), |c| format!("#{}", c.name))
    }
    fn send_later_config(&self, cx: &App) -> components::SendLaterConfig {
        components::SendLaterConfig {
            presets: settings::SendLaterSettings::default().presets,
            zone: self.schedule_zone(cx),
            clock: settings::Clock::H24,
            server: self.caps.scheduled_send,
        }
    }
}

impl components::ChannelDetailsSource for ScenarioSession {
    fn details_state(&self) -> &WorkspaceState {
        &self.state
    }
    fn details_conversation(&self) -> Option<&ConversationId> {
        self.selected.as_ref()
    }
}

impl components::ProfileSource for ScenarioSession {
    fn profile(&self, u: &UserId) -> Option<&domain::Profile> {
        self.state.profiles.get(u)
    }
    fn dnd(&self, u: &UserId) -> Option<&domain::DndStatus> {
        self.state.dnd.get(u)
    }
    fn in_huddle(&self, u: &UserId) -> bool {
        self.state.in_huddle.contains(u)
    }
    fn shown_profile(&self) -> Option<&UserId> {
        self.state.open_profile.as_ref()
    }
    fn is_own(&self, u: &UserId) -> bool {
        self.state.self_user.as_ref() == Some(u)
    }
    fn own_presence(&self) -> Option<domain::PresenceSetting> {
        self.own_presence
    }
    fn has_dm(&self, u: &UserId) -> bool {
        self.state
            .conversations
            .values()
            .any(|c| c.kind == domain::ConversationKind::Direct && c.peer.as_ref() == Some(u))
    }
    fn presence(&self, u: &UserId) -> Option<bool> {
        self.state.presence.get(u).copied()
    }
    fn can_set_status(&self) -> bool {
        self.caps.status
    }
    fn can_set_presence(&self) -> bool {
        self.caps.presence
    }
}

impl components::CollectionsSource for ScenarioSession {
    fn pins(&self, _: &ConversationId) -> Option<&[domain::Pin]> {
        self.collections.as_ref()?.pins.as_deref()
    }
    fn bookmarks(&self, _: &ConversationId) -> Option<&[domain::Bookmark]> {
        self.collections.as_ref()?.bookmarks.as_deref()
    }
    fn saved(&self) -> Option<&domain::Page<domain::SavedItem>> {
        self.collections.as_ref()?.saved.as_ref()
    }
    fn thread_inbox(&self) -> Option<&domain::Page<domain::InboxThread>> {
        self.collections.as_ref()?.inbox.as_ref()
    }
    fn referenced(&self, id: &MessageId) -> Option<&Message> {
        self.collections
            .as_ref()?
            .held
            .iter()
            .find(|m| m.id == *id)
            .or_else(|| {
                self.state
                    .windows
                    .get(&id.conversation)?
                    .messages()
                    .iter()
                    .find(|m| m.id == *id)
            })
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn conversation_label(&self, c: &ConversationId, cx: &App) -> String {
        components::ScheduledSource::conversation_label(self, c, cx)
    }
    fn content(&self) -> Arc<dyn ContentParser> {
        Arc::new(ScenarioParser::new(&self.state))
    }
    fn time_config(&self, cx: &App) -> components::SendLaterConfig {
        components::ScheduledSource::send_later_config(self, cx)
    }
    fn dispatch(&mut self, _: Input, _: &mut App) {}
}

impl components::RemindersSource for ScenarioSession {
    fn reminders(&self) -> Option<&[domain::Reminder]> {
        self.reminders.as_ref().and_then(|r| r.items.as_deref())
    }
    fn complete(&mut self, _: domain::ReminderId, _: &mut App) {}
    fn delete(&mut self, _: domain::ReminderId, _: &mut App) {}
    fn add(&mut self, _: String, _: String, _: &mut App) {}
    fn local(&self) -> bool {
        self.reminders.as_ref().is_some_and(|r| r.local)
    }
    fn when_at(&self, when: &str, cx: &App) -> Option<i64> {
        let zone = components::ScheduledSource::send_later_config(self, cx).zone;
        domain::parse_when(when, &components::now(cx).to_zoned_datetime(zone)).map(|d| d.at)
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn conversation_label(&self, c: &ConversationId, cx: &App) -> String {
        components::ScheduledSource::conversation_label(self, c, cx)
    }
    fn time_config(&self, cx: &App) -> components::SendLaterConfig {
        components::ScheduledSource::send_later_config(self, cx)
    }
}

impl components::PeopleSource for ScenarioSession {
    fn people_state(&self) -> &WorkspaceState {
        &self.state
    }
    fn members_revision(&self) -> u64 {
        1
    }
    fn clock(&self, _cx: &App) -> settings::Clock {
        settings::Clock::H24
    }
}

impl components::DraftsSource for ScenarioSession {
    fn draft_items(&self, _cx: &App) -> Vec<components::DraftItem> {
        self.saved_drafts
            .as_ref()
            .map(|d| d.items.clone())
            .unwrap_or_default()
    }
    fn delete_draft(&mut self, _key: domain::DraftKey, _cx: &mut App) {}
}

impl SidebarSource for ScenarioSession {
    fn people_entry(&self) -> bool {
        self.people
    }
    fn workspace_name(&self, _cx: &App) -> Option<SharedString> {
        let name = self.state.name.as_ref();
        (!name.is_empty()).then(|| SharedString::from(name.to_string()))
    }
    fn threads_entry(&self) -> Option<usize> {
        let inbox = self.collections.as_ref()?.inbox.as_ref()?;
        Some(inbox.items.iter().map(|t| t.unread_replies as usize).sum())
    }
    fn later_entry(&self) -> Option<usize> {
        let saved = self.collections.as_ref()?.saved.as_ref()?;
        Some(saved.items.iter().filter(|i| !i.done).count())
    }
    fn reminders_entry(&self) -> Option<usize> {
        self.reminders.as_ref().map(|r| {
            r.items
                .as_ref()
                .map_or(0, |all| all.iter().filter(|x| !x.completed).count())
        })
    }
    fn scheduled_entry(&self) -> Option<usize> {
        self.scheduled
            .as_ref()
            .map(|s| s.server.len() + s.local.len())
    }
    fn rows(&self, cx: &App) -> Vec<SidebarRow> {
        build_rows_sections(
            &self.state,
            &MemberIndex::build(&self.state),
            self.selected.as_ref(),
            None,
            &self.sidebar_settings,
            &self.drafts,
            i18n::Translate::localizer(cx),
            self.dms_expanded,
            &self.collapsed,
        )
    }

    fn drafts_entry(&self) -> Option<usize> {
        self.saved_drafts
            .as_ref()
            .map(|d| d.items.len())
            .filter(|n| *n > 0)
    }

    fn open(&mut self, _id: ConversationId, _cx: &mut App) {}

    fn paused_label(&self, cx: &App) -> Option<String> {
        let cfg = components::NotifyControls::notify_config(self, cx);
        let ends = components::NotifyControls::paused(self, components::now_secs(cx), &cfg.zone)?;
        Some(components::paused_text(
            &ends,
            &cfg,
            i18n::Translate::localizer(cx),
        ))
    }

    fn toggle_direct_messages(&mut self, _cx: &mut App) {
        self.dms_expanded = !self.dms_expanded;
    }
}

impl components::HuddleSource for ScenarioSession {
    fn huddle(&self, c: &ConversationId) -> Option<&domain::Huddle> {
        self.state.huddles.get(c)
    }
    fn self_user(&self) -> Option<&UserId> {
        self.state.self_user.as_ref()
    }
    fn member(&self, u: &UserId) -> Option<&domain::Member> {
        self.state.members.get(u)
    }
    fn join_huddle(&mut self, _: ConversationId, _: &mut App) {}
    fn can_start_huddle(&self) -> bool {
        self.can_start_huddle
    }
    fn in_call_here(&self, _: &ConversationId, _: &App) -> bool {
        self.in_call
    }
}

impl TimelineSource for ScenarioSession {
    fn offline(&self) -> bool {
        self.state.connection == domain::Connection::Offline
    }
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn thread_open(&self, root: &MessageId) -> bool {
        self.state.threads.contains_key(root)
    }
    fn history_failed(&self, _: &ConversationId) -> bool {
        self.history_failed
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn self_user(&self) -> Option<&UserId> {
        self.state.self_user.as_ref()
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn load_older(&mut self, _c: ConversationId, _cx: &mut App) {}
    fn huddle_live(&self, c: &ConversationId) -> bool {
        self.state.huddles.contains_key(c)
    }
    fn jump(&self) -> Option<&MessageId> {
        self.state.jump.as_ref()
    }
    fn edit(&mut self, _id: MessageId, _text: String, _cx: &mut App) {}
    fn delete(&mut self, _id: MessageId, _cx: &mut App) {}
    fn react(&mut self, _id: MessageId, _emoji: EmojiRef, _add: bool, _cx: &mut App) {}
    fn custom_emoji(&self) -> Vec<CustomEmoji> {
        self.state.custom_emoji.values().cloned().collect()
    }
    fn content(&self) -> Arc<dyn ContentParser> {
        Arc::new(ScenarioParser::new(&self.state))
    }
    fn conversation(&self, id: &ConversationId) -> Option<&Conversation> {
        self.state.conversations.get(id)
    }
    fn workspace_state(&self) -> Option<&domain::WorkspaceState> {
        Some(&self.state)
    }
    fn download(&mut self, _file: &FileRef, _cx: &mut App) {}
    fn web_url(&self, id: &MessageId) -> Option<String> {
        self.collections.as_ref().map(|_| {
            format!(
                "https://example.com/archives/{}/p{}",
                id.conversation.0, id.ts.0
            )
        })
    }
    fn user_group(&self, id: &str) -> Option<&domain::UserGroup> {
        self.state.user_groups.get(id)
    }
}

impl ComposerSource for ScenarioSession {
    /// The scenario's typers, without their expiry: a render takes no time.
    fn typing(
        &self,
        c: &ConversationId,
        thread: Option<&domain::MessageTs>,
    ) -> Vec<Option<&Member>> {
        self.state
            .typers(c, thread)
            .iter()
            .map(|t| self.state.members.get(&t.user))
            .collect()
    }
    fn picker_custom_emoji(&self) -> Vec<CustomEmoji> {
        self.state.custom_emoji.values().cloned().collect()
    }
    fn custom_emoji_image(&self, shortcode: &str) -> Option<domain::ImageRef> {
        self.state
            .custom_emoji
            .get(shortcode)
            .map(|e| e.image.clone())
    }

    fn read_only(&self) -> bool {
        self.state.session == domain::Session::Expired
    }

    /// A public channel the user is not in, when the provider can join it
    /// (the app's rule).
    fn must_join(&self, c: &ConversationId) -> bool {
        self.state.caps.channel_admin
            && self.state.conversations.get(c).is_some_and(|c| {
                !c.is_member && !c.is_archived && c.kind == domain::ConversationKind::Channel
            })
    }

    fn staged(
        &self,
        _c: &ConversationId,
        thread: Option<&domain::MessageTs>,
    ) -> Vec<components::StagedFile> {
        use components::StagedState;
        if thread.is_some() {
            return Vec::new();
        }
        self.uploads
            .iter()
            .map(|(id, name, st)| components::StagedFile {
                id: *id,
                name: name.clone(),
                size: staged_size(name),
                thumb: None,
                snippet: None,
                state: match st {
                    UploadState::Ready => StagedState::Ready,
                    UploadState::Progress(p) => StagedState::Uploading(Some(*p)),
                    UploadState::Uploading => StagedState::Uploading(None),
                    UploadState::Failed => StagedState::Failed("network error".into()),
                    UploadState::Cancelled => StagedState::Cancelled,
                },
            })
            .collect()
    }
    fn send_later(&self, cx: &App) -> components::SendLaterConfig {
        components::ScheduledSource::send_later_config(self, cx)
    }
    fn completions(
        &self,
        kind: components::CompletionKind,
        query: &str,
    ) -> Vec<components::Completion> {
        use components::{CompletionKind as K, completion};
        match kind {
            K::Person => completion::people(&self.state, None, query),
            K::Conversation => completion::conversations(&self.state, query),
            K::Emoji => completion::emoji(self.state.custom_emoji.values(), query),
        }
    }
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn self_user(&self) -> Option<&UserId> {
        self.state.self_user.as_ref()
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
}

impl ThreadSource for ScenarioSession {
    fn thread(&self, root: &MessageId) -> Option<&domain::ThreadWindow> {
        self.state.threads.get(root)
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn replies_failed(&self, _: &MessageId) -> bool {
        self.replies_failed
    }
    fn open_thread(&mut self, _root: MessageId, _cx: &mut App) {}
    fn close_thread(&mut self, _root: MessageId, _cx: &mut App) {}
    fn reply(&mut self, _root: MessageId, _text: String, _broadcast: bool, _cx: &mut App) {}
    fn react(&mut self, _id: MessageId, _emoji: EmojiRef, _add: bool, _cx: &mut App) {}
    fn thread_follow(&self, _root: &MessageId) -> Option<bool> {
        self.followed
    }
    fn custom_emoji_image(&self, shortcode: &str) -> Option<domain::ImageRef> {
        self.state
            .custom_emoji
            .get(shortcode)
            .map(|e| e.image.clone())
    }
}

impl components::SearchSource for ScenarioSession {
    fn search(&mut self, _scope: components::SearchScope, _input: String, _cx: &mut App) {}
    fn search_results(&self) -> &components::SearchResults {
        &self.search
    }
    fn remote_search(&self) -> bool {
        self.remote_search
    }
    fn provider_name(&self) -> &str {
        "Example"
    }
    fn context(&self, id: &MessageId, cx: &App) -> Vec<components::PreviewLine> {
        let Some(msgs) = self
            .state
            .windows
            .get(&id.conversation)
            .and_then(|w| components::context_around(w.messages(), id))
        else {
            return Vec::new();
        };
        let tz = components::time_zone(cx);
        let l = i18n::gpui::localizer(cx);
        let parser = domain::PlainParser;
        msgs.iter()
            .map(|m| {
                let author = m.author.as_ref().and_then(|u| self.state.members.get(u));
                components::preview_line(
                    m,
                    author,
                    &parser,
                    settings::Clock::H24,
                    &tz,
                    &l,
                    &self.state.user_groups,
                )
            })
            .collect()
    }
}

/// The data of a cross-workspace scenario, behind the view's source trait.
pub struct AggregateFixture {
    pub data: crate::scenario::Aggregate,
}

impl components::AggregateSource for AggregateFixture {
    fn unreads(&self, _: &App) -> Vec<components::UnreadItem> {
        self.data.unreads.clone()
    }
    fn mentions(&self, _: &App) -> Vec<components::MentionItem> {
        self.data.mentions.clone()
    }
    fn scheduled(&self, _: &App) -> Vec<components::ScheduledItem> {
        self.data.scheduled.clone()
    }
    fn mark_all_read(&mut self, _: &mut App) {}
    fn mark_read(
        &mut self,
        _: &settings::WorkspaceKey,
        _: &ConversationId,
        _: domain::MessageTs,
        _: &mut App,
    ) {
    }
    fn previews(
        &self,
        _: &settings::WorkspaceKey,
        c: &ConversationId,
        _: &App,
    ) -> Vec<components::PreviewLine> {
        self.data
            .previews
            .iter()
            .find(|(at, _)| at == c)
            .map(|(_, l)| l.clone())
            .unwrap_or_default()
    }
    fn config(&self, cx: &App) -> components::SendLaterConfig {
        components::SendLaterConfig {
            presets: settings::SendLaterSettings::default().presets,
            zone: components::time_zone(cx),
            clock: settings::Clock::H24,
            server: true,
        }
    }
}

impl components::NotifyControls for ScenarioSession {
    fn can_pause(&self) -> bool {
        self.caps.dnd
    }
    fn paused(&self, now: i64, tz: &jiff::tz::TimeZone) -> Option<domain::DndEnds> {
        let me = self.state.self_user.as_ref()?;
        domain::dnd_ends(self.state.dnd.get(me)?, now, tz)
    }
    fn conversation_prefs(&self, c: &ConversationId) -> domain::ConversationPrefs {
        self.state.prefs_for(c).copied().unwrap_or_default()
    }
    fn notify_extras(&self, c: &ConversationId) -> domain::NotifyExtras {
        self.state
            .local_notify_extras
            .get(c)
            .copied()
            .unwrap_or_default()
    }
    fn notify_config(&self, cx: &App) -> components::SendLaterConfig {
        components::SendLaterConfig {
            presets: Vec::new(),
            zone: self.schedule_zone(cx),
            clock: settings::Clock::H24,
            server: false,
        }
    }
}

/// Sizes shown on the staged chips, fixed per name so the baselines are stable.
fn staged_size(name: &str) -> u64 {
    match name.rsplit('.').next() {
        Some("mov") => 1_900_000,
        Some("json") => 8_192,
        Some("png") => 48_000,
        _ => 12_345,
    }
}

/// The Files tab lists the files of the open conversation's loaded messages,
/// newest first, answering on the next effect cycle like a store read.
impl components::FilesSource for ScenarioSession {
    fn files_state(&self) -> &WorkspaceState {
        &self.state
    }
    fn load_files(
        &mut self,
        c: ConversationId,
        _cursor: Option<Arc<str>>,
        limit: usize,
        done: components::FilesDone,
        cx: &mut App,
    ) {
        let items: Vec<domain::FileHit> = self
            .state
            .windows
            .get(&c)
            .map(|w| {
                w.messages()
                    .iter()
                    .rev()
                    .flat_map(|m| {
                        m.files.iter().map(|f| domain::FileHit {
                            file: f.clone(),
                            conversation: Some(c.clone()),
                            author: m.author.clone(),
                            ts: Some(m.id.ts.clone()),
                        })
                    })
                    .take(limit)
                    .collect()
            })
            .unwrap_or_default();
        cx.defer(move |cx| done(Ok(domain::Page { items, next: None }), cx));
    }
    fn view_file(&mut self, _gallery: components::media_viewer::Gallery, _cx: &mut App) {}
    fn download_file(&mut self, _file: &domain::FileRef, _cx: &mut App) {}
}
