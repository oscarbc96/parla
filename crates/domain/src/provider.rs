use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use settings::{ProviderId, WorkspaceKey};
use url::Url;

use crate::accounts::AccountInfo;
use crate::call_surface::{CallSurface, NativeCallJoin};
use crate::caps::Capabilities;
use crate::compose::{ContentEncoder, PlainEncoder};
use crate::content::{ContentParser, ExtCodec};
use crate::files::Upload;
use crate::huddle::{Huddle, HuddleId};
use crate::ids::{ClientMsgId, ConversationId, MessageId, MessageTs, RemoteUrl, UserId};
use crate::model::{
    Bootstrap, Conversation, HistoryPage, Member, Message, ProviderError, ProviderEvent,
};
use crate::people::{ConversationPrefs, DndStatus, PresenceSetting, Profile, Status};
use crate::reactions::CustomEmoji;
use crate::reminder::{Reminder, ReminderId};
use crate::rich::EmojiRef;
use crate::saved::{Bookmark, BookmarkId, InboxThread, Pin, SavedItem};
use crate::schedule::ScheduledMessage;
use crate::search::{AroundPage, FileHit, MessageHit, Page, SearchQuery};
use crate::sync_types::ConversationCounts;

/// A provider permalink resolved to a neutral conversation target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPermalink {
    /// Provider-defined hint used to match the signed-in workspace (for
    /// example the workspace host or domain).
    pub workspace_hint: Arc<str>,
    pub conversation: ConversationId,
    pub ts: Option<MessageTs>,
    pub thread: Option<MessageTs>,
}

/// Minimal on purpose; reshaped when a second provider arrives.
pub trait ChatProvider: Send + Sync + 'static {
    fn workspace(&self) -> &WorkspaceKey;
    fn capabilities(&self) -> &Capabilities;
    fn bootstrap(&self) -> BoxFuture<'_, Result<Bootstrap, ProviderError>>;
    fn history(
        &self,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: u32,
    ) -> BoxFuture<'_, Result<HistoryPage, ProviderError>>;
    fn send(
        &self,
        c: &ConversationId,
        text: &str,
        thread: Option<MessageTs>,
        id: ClientMsgId,
    ) -> BoxFuture<'_, Result<Message, ProviderError>>;
    fn edit(&self, id: &MessageId, text: &str) -> BoxFuture<'_, Result<(), ProviderError>>;
    fn delete(&self, id: &MessageId) -> BoxFuture<'_, Result<(), ProviderError>>;
    fn mark_read(
        &self,
        c: &ConversationId,
        ts: &MessageTs,
    ) -> BoxFuture<'_, Result<(), ProviderError>>;
    fn events(&self) -> BoxStream<'static, ProviderEvent>;

    /// Newest first, strictly between `after` and `before`; paging continues
    /// with `before` = the oldest ts received.
    fn history_between(
        &self,
        c: &ConversationId,
        after: MessageTs,
        before: Option<MessageTs>,
        limit: u32,
    ) -> BoxFuture<'_, Result<HistoryPage, ProviderError>>;
    /// Ascending, root first when `after` is `None`.
    fn replies(
        &self,
        c: &ConversationId,
        root: &MessageTs,
        after: Option<MessageTs>,
        limit: u32,
    ) -> BoxFuture<'_, Result<HistoryPage, ProviderError>>;
    fn reply(
        &self,
        c: &ConversationId,
        root: &MessageTs,
        text: &str,
        broadcast: bool,
        id: ClientMsgId,
    ) -> BoxFuture<'_, Result<Message, ProviderError>>;
    fn react(
        &self,
        id: &MessageId,
        emoji: &EmojiRef,
        add: bool,
    ) -> BoxFuture<'_, Result<(), ProviderError>>;
    fn mark_thread_read(
        &self,
        c: &ConversationId,
        root: &MessageTs,
        ts: &MessageTs,
    ) -> BoxFuture<'_, Result<(), ProviderError>>;
    fn counts(&self) -> BoxFuture<'_, Result<Vec<ConversationCounts>, ProviderError>>;
    fn custom_emoji(&self) -> BoxFuture<'_, Result<Vec<CustomEmoji>, ProviderError>>;
    fn fetch(
        &self,
        url: &RemoteUrl,
        max_bytes: u64,
    ) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>>;
    fn download(&self, url: &RemoteUrl, dest: &Path) -> BoxFuture<'_, Result<u64, ProviderError>>;
    fn upload(
        &self,
        c: &ConversationId,
        thread: Option<MessageTs>,
        files: Vec<Upload>,
        text: Option<&str>,
        id: ClientMsgId,
    ) -> BoxFuture<'_, Result<Message, ProviderError>>;
    /// Permalink to open in a browser, when the provider has one.
    fn web_url(&self, id: &MessageId) -> Option<String>;
    /// Link to conversation `c` in the provider's own client, when it has
    /// one (the row menu's Copy link).
    fn conversation_web_url(&self, _c: &ConversationId) -> Option<String> {
        None
    }
    /// The workspace's web origin or base URL, when the provider knows one.
    /// Used to match inbound permalinks to a signed-in workspace.
    fn web_origin(&self) -> Option<String> {
        None
    }

    /// Skips the wait before the next automatic reconnect attempt. A no-op
    /// when connected, or for a provider that has no connection to retry.
    fn reconnect_now(&self) {}

    /// Tells the others in `conversation` (its thread `thread` when set) that
    /// the user is typing. Fire and forget, already throttled by the caller;
    /// needs `Capabilities::typing`, a no-op otherwise.
    fn typing(&self, _conversation: &ConversationId, _thread: Option<&MessageTs>) {}

    // ---- M3. Every method below defaults to `Unsupported`; a provider opts
    // in by implementing it and setting the matching `Capabilities` flag.

    fn search_messages(
        &self,
        _q: &SearchQuery,
        _cursor: Option<Arc<str>>,
        _limit: u32,
    ) -> BoxFuture<'_, Result<Page<MessageHit>, ProviderError>> {
        unsupported()
    }
    fn search_files(
        &self,
        _q: &SearchQuery,
        _cursor: Option<Arc<str>>,
        _limit: u32,
    ) -> BoxFuture<'_, Result<Page<FileHit>, ProviderError>> {
        unsupported()
    }
    fn search_people(
        &self,
        _text: &str,
        _limit: u32,
    ) -> BoxFuture<'_, Result<Vec<Member>, ProviderError>> {
        unsupported()
    }
    fn search_conversations(
        &self,
        _text: &str,
        _limit: u32,
    ) -> BoxFuture<'_, Result<Vec<Conversation>, ProviderError>> {
        unsupported()
    }
    /// Messages around `ts` (ascending, `ts` included): `limit / 2` up to and
    /// including `ts`, then `limit / 2` after it.
    ///
    /// `history_between` answers newest first, so the messages right after
    /// `ts` are only reachable by paging down from the newest; the default
    /// pages at most [`AROUND_MAX_PAGES`] times and fails beyond that. A
    /// provider with an oldest-anchored read overrides this.
    fn history_around(
        &self,
        c: &ConversationId,
        ts: &MessageTs,
        limit: u32,
    ) -> BoxFuture<'_, Result<AroundPage, ProviderError>> {
        let (c, ts) = (c.clone(), ts.clone());
        Box::pin(async move {
            let half = (limit / 2).max(1);
            let upto = self.history(&c, Some(ts.just_after()), half).await?;
            let mut after: Vec<Message> = Vec::new();
            let mut before_bound: Option<MessageTs> = None;
            let mut pages = 0;
            loop {
                let page = self
                    .history_between(&c, ts.clone(), before_bound.clone(), half)
                    .await?;
                before_bound = page.messages.iter().map(|m| m.id.ts.clone()).min();
                let more = page.has_more && before_bound.is_some();
                after.extend(page.messages);
                if !more {
                    break;
                }
                pages += 1;
                if pages >= AROUND_MAX_PAGES {
                    return Err(ProviderError::Other(
                        "message is too far from the newest one".into(),
                    ));
                }
            }
            after.sort_by(|a, b| a.id.ts.cmp(&b.id.ts));
            let has_more_after = after.len() > half as usize;
            after.truncate(half as usize);
            let mut messages = upto.messages;
            messages.extend(after);
            Ok(AroundPage {
                messages,
                has_more_before: upto.has_more,
                has_more_after,
            })
        })
    }
    fn schedule(
        &self,
        _c: &ConversationId,
        _thread: Option<MessageTs>,
        _text: &str,
        _post_at: i64,
    ) -> BoxFuture<'_, Result<ScheduledMessage, ProviderError>> {
        unsupported()
    }
    fn scheduled(&self) -> BoxFuture<'_, Result<Vec<ScheduledMessage>, ProviderError>> {
        unsupported()
    }
    fn delete_scheduled(&self, _s: &ScheduledMessage) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    fn reminders(&self) -> BoxFuture<'_, Result<Vec<Reminder>, ProviderError>> {
        unsupported()
    }
    fn add_reminder(
        &self,
        _text: &str,
        _when: &str,
        _user: Option<&UserId>,
    ) -> BoxFuture<'_, Result<Reminder, ProviderError>> {
        unsupported()
    }
    fn complete_reminder(&self, _id: &ReminderId) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    fn delete_reminder(&self, _id: &ReminderId) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    fn profile(&self, _u: &UserId) -> BoxFuture<'_, Result<Profile, ProviderError>> {
        unsupported()
    }
    fn set_status(&self, _s: Option<Status>) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    fn set_presence(&self, _p: PresenceSetting) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    /// Replaces the set of people whose presence changes are pushed (those on
    /// screen). Needs `Capabilities::presence_watch`.
    fn watch_presence(&self, _users: &[UserId]) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    /// Opens (or creates) the one-to-one direct message with `user` and returns
    /// it. Needs `Capabilities::open_direct`.
    fn open_direct(&self, _user: &UserId) -> BoxFuture<'_, Result<Conversation, ProviderError>> {
        unsupported()
    }
    /// The signed-in user's own do-not-disturb state.
    fn dnd(&self) -> BoxFuture<'_, Result<DndStatus, ProviderError>> {
        unsupported()
    }
    fn dnd_users(
        &self,
        _users: &[UserId],
    ) -> BoxFuture<'_, Result<HashMap<UserId, DndStatus>, ProviderError>> {
        unsupported()
    }
    fn snooze(&self, _minutes: u32) -> BoxFuture<'_, Result<DndStatus, ProviderError>> {
        unsupported()
    }
    fn end_snooze(&self) -> BoxFuture<'_, Result<DndStatus, ProviderError>> {
        unsupported()
    }
    fn notify_prefs(
        &self,
    ) -> BoxFuture<'_, Result<HashMap<ConversationId, ConversationPrefs>, ProviderError>> {
        unsupported()
    }
    fn set_notify_prefs(
        &self,
        _c: &ConversationId,
        _p: ConversationPrefs,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    fn huddles(&self) -> BoxFuture<'_, Result<Vec<Huddle>, ProviderError>> {
        unsupported()
    }
    fn decline_huddle(
        &self,
        _c: &ConversationId,
        _id: &HuddleId,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    /// One message by id.
    fn message(&self, _id: &MessageId) -> BoxFuture<'_, Result<Message, ProviderError>> {
        unsupported()
    }
    /// One conversation by id (`Capabilities::conversation_lookup`), for a live
    /// change to one the app does not hold yet.
    fn conversation(
        &self,
        _c: &ConversationId,
    ) -> BoxFuture<'_, Result<Conversation, ProviderError>> {
        unsupported()
    }
    fn pins(&self, _c: &ConversationId) -> BoxFuture<'_, Result<Vec<Pin>, ProviderError>> {
        unsupported()
    }
    fn set_pinned(
        &self,
        _id: &MessageId,
        _pinned: bool,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    fn bookmarks(
        &self,
        _c: &ConversationId,
    ) -> BoxFuture<'_, Result<Vec<Bookmark>, ProviderError>> {
        unsupported()
    }
    fn add_bookmark(
        &self,
        _c: &ConversationId,
        _title: &str,
        _link: &str,
    ) -> BoxFuture<'_, Result<Bookmark, ProviderError>> {
        unsupported()
    }
    fn remove_bookmark(
        &self,
        _c: &ConversationId,
        _id: &BookmarkId,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    fn saved(
        &self,
        _cursor: Option<Arc<str>>,
    ) -> BoxFuture<'_, Result<Page<SavedItem>, ProviderError>> {
        unsupported()
    }
    fn set_saved(&self, _id: &MessageId, _saved: bool) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    fn thread_inbox(
        &self,
        _cursor: Option<Arc<str>>,
    ) -> BoxFuture<'_, Result<Page<InboxThread>, ProviderError>> {
        unsupported()
    }
    /// Follows (`follow`) or unfollows the thread under `root`: whether its
    /// replies reach the user. `last_read` is the user's read cursor in the thread when
    /// known. Needs `Capabilities::thread_follow`.
    fn set_thread_followed(
        &self,
        _root: &MessageId,
        _follow: bool,
        _last_read: Option<&MessageTs>,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    /// Members by id, for authors the bootstrap directory did not carry.
    /// Ids the provider does not know are left out of the answer.
    fn members(&self, _ids: &[UserId]) -> BoxFuture<'_, Result<Vec<Member>, ProviderError>> {
        unsupported()
    }
    /// The workspace's user groups (`Capabilities::user_groups`), for `@`
    /// completion and mentions of groups.
    fn user_groups(
        &self,
    ) -> BoxFuture<'_, Result<Vec<crate::user_groups::UserGroup>, ProviderError>> {
        unsupported()
    }
    // ---- Channel admin (`Capabilities::channel_admin`).

    /// Creates a channel named `name` (already normalized); `private` makes it
    /// invite-only. Answers the new conversation, the user a member.
    fn create_channel(
        &self,
        _name: &str,
        _private: bool,
    ) -> BoxFuture<'_, Result<Conversation, ProviderError>> {
        unsupported()
    }
    /// Channels the user can see, members or not, matching `query` (all when
    /// blank), a page at a time; `purpose` and `member_count` filled when known.
    fn browse_channels(
        &self,
        _query: &str,
        _cursor: Option<Arc<str>>,
        _limit: u32,
    ) -> BoxFuture<'_, Result<Page<Conversation>, ProviderError>> {
        unsupported()
    }
    /// Joins a channel; answers it with the user a member.
    fn join_conversation(
        &self,
        _c: &ConversationId,
    ) -> BoxFuture<'_, Result<Conversation, ProviderError>> {
        unsupported()
    }
    fn leave_conversation(&self, _c: &ConversationId) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    fn archive_conversation(
        &self,
        _c: &ConversationId,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    fn rename_conversation(
        &self,
        _c: &ConversationId,
        _name: &str,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    fn set_topic(
        &self,
        _c: &ConversationId,
        _topic: &str,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    /// Sets the description (what the channel is for).
    fn set_purpose(
        &self,
        _c: &ConversationId,
        _purpose: &str,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    /// Invites people; the answer says who was added and why the others
    /// were not.
    fn invite(
        &self,
        _c: &ConversationId,
        _users: &[UserId],
    ) -> BoxFuture<'_, Result<crate::InviteReport, ProviderError>> {
        unsupported()
    }
    /// Everyone in a channel (all pages), for the invite picker.
    fn conversation_members(
        &self,
        _c: &ConversationId,
    ) -> BoxFuture<'_, Result<Vec<UserId>, ProviderError>> {
        unsupported()
    }
    /// The name `typed` as the provider would store it and what is wrong with
    /// it. The default only trims and refuses a blank name.
    fn check_channel_name(&self, typed: &str) -> crate::ChannelName {
        let name = typed.trim().to_string();
        crate::ChannelName {
            problem: name.is_empty().then_some(crate::NameProblem::Empty),
            name,
        }
    }
    /// Stars or unstars a conversation on the provider
    /// (`Capabilities::conversation_stars`).
    fn set_conversation_starred(
        &self,
        _c: &ConversationId,
        _starred: bool,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    /// Where the provider's huddle for `c` can be joined in a browser.
    fn huddle_url(&self, _c: &ConversationId) -> Option<String> {
        None
    }
    /// A URL that hands the conversation to the provider's own app.
    fn app_url(&self, _c: &ConversationId) -> Option<String> {
        None
    }
    /// What the call window needs to open the huddle for `c`.
    fn call_surface(
        &self,
        _c: &ConversationId,
    ) -> BoxFuture<'_, Result<CallSurface, ProviderError>> {
        unsupported()
    }
    /// What a native client needs to join the call for `c` (default: unsupported).
    fn native_call_join(
        &self,
        _c: &ConversationId,
    ) -> BoxFuture<'_, Result<NativeCallJoin, ProviderError>> {
        unsupported()
    }
    /// Tells the provider we left the huddle for `c` (default: nothing to do).
    fn leave_huddle(&self, _c: &ConversationId) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(std::future::ready(Ok(())))
    }
}

/// Most pages `history_around`'s default reads past the target.
pub const AROUND_MAX_PAGES: u32 = 8;

fn unsupported<'a, T: Send + 'a>() -> BoxFuture<'a, Result<T, ProviderError>> {
    Box::pin(std::future::ready(Err(ProviderError::Unsupported)))
}

/// What the registry holds, one per provider.
pub trait ProviderFactory: Send + Sync {
    fn id(&self) -> &ProviderId;
    fn display_name(&self) -> &str;
    fn accounts(&self) -> Vec<WorkspaceKey>;
    fn start(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Arc<dyn ChatProvider>, ProviderError>>;
    fn ext_codec(&self) -> Arc<dyn ExtCodec>;
    fn content_parser(&self) -> Arc<dyn ContentParser>;
    /// The provider's message catalog (error texts, the unofficial notice
    /// under the id `unofficial-notice`); the shell registers it with the
    /// localizer.
    fn catalog(&self) -> &'static i18n::Catalog;
    /// One entry per account, for the workspace rail and the account list
    /// (default: the account id as the name).
    fn account_info(&self) -> Vec<AccountInfo> {
        self.accounts()
            .into_iter()
            .map(|key| AccountInfo {
                name: Arc::from(key.id.0.as_str()),
                key,
                org: None,
                icon: None,
            })
            .collect()
    }
    /// Deletes the provider's secrets and account row for `ws`.
    fn remove(&self, _ws: &WorkspaceKey) -> BoxFuture<'_, Result<(), ProviderError>> {
        unsupported()
    }
    /// Encodes composed text into the provider's wire markup.
    fn encoder(&self) -> Arc<dyn ContentEncoder> {
        Arc::new(PlainEncoder)
    }
    /// Parses an https provider permalink into a neutral target. `None` when
    /// this provider does not own the URL.
    fn parse_permalink(&self, _url: &Url) -> Option<ParsedPermalink> {
        None
    }
}
