//! Method names and their rate-limit tiers. Constants are added per task.

/// Slack rate-limit tier of a method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    T1,
    T2,
    T3,
    T4,
    PostMessage,
}

/// Who asked for the call; the limiter uses it to protect user actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    User,
    Visible,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Method {
    pub name: &'static str,
    pub tier: Tier,
}

/// Declares a [`Method`] constant: `method!(NAME, "family.method", T3);`. Doc comments go
/// inside the call, before the name.
macro_rules! method {
    ($(#[$meta:meta])* $name:ident, $wire:literal, $tier:ident) => {
        $(#[$meta])*
        pub const $name: Method = Method {
            name: $wire,
            tier: Tier::$tier,
        };
    };
}

method!(AUTH_TEST, "auth.test", T4);

method!(TEAM_INFO, "team.info", T3);

method!(USERS_CONVERSATIONS, "users.conversations", T3);
method!(CONVERSATIONS_HISTORY, "conversations.history", T3);
method!(CONVERSATIONS_INFO, "conversations.info", T3);
method!(
    /// Unverified against a capture (public Web API method).
    CONVERSATIONS_OPEN,
    "conversations.open",
    T3
);
method!(USERS_INFO, "users.info", T4);
method!(BOTS_INFO, "bots.info", T3);
method!(USERS_LIST, "users.list", T2);

method!(CLIENT_USER_BOOT, "client.userBoot", T2);
method!(CLIENT_COUNTS, "client.counts", T3);
method!(CLIENT_SHOULD_RELOAD, "client.shouldReload", T3);

method!(CHAT_POST_MESSAGE, "chat.postMessage", PostMessage);
method!(CHAT_UPDATE, "chat.update", T3);
method!(CHAT_DELETE, "chat.delete", T3);
method!(CONVERSATIONS_MARK, "conversations.mark", T2);

// Channel admin, seen in the 2026-11-24 web-client capture (`conversations.invite` is
// assumed from the public Web API: inviting was not captured). Tiers are the public ones.
method!(
    CONVERSATIONS_VALIDATE_NAME,
    "conversations.validateName",
    T3
);
method!(CONVERSATIONS_CREATE, "conversations.create", T2);
method!(CONVERSATIONS_JOIN, "conversations.join", T3);
method!(CONVERSATIONS_LEAVE, "conversations.leave", T3);
method!(CONVERSATIONS_ARCHIVE, "conversations.archive", T2);
method!(CONVERSATIONS_RENAME, "conversations.rename", T2);
method!(CONVERSATIONS_SET_TOPIC, "conversations.setTopic", T3);
method!(CONVERSATIONS_SET_PURPOSE, "conversations.setPurpose", T3);
method!(CONVERSATIONS_INVITE, "conversations.invite", T3);
method!(CONVERSATIONS_MEMBERS, "conversations.members", T4);
method!(STARS_ADD, "stars.add", T2);
method!(STARS_REMOVE, "stars.remove", T2);

method!(CONVERSATIONS_REPLIES, "conversations.replies", T3);
method!(REACTIONS_ADD, "reactions.add", T3);
method!(REACTIONS_REMOVE, "reactions.remove", T3);
method!(EMOJI_LIST, "emoji.list", T2);
method!(USERGROUPS_LIST, "usergroups.list", T2);
method!(SUBSCRIPTIONS_THREAD_MARK, "subscriptions.thread.mark", T3);
// Thread follow, recorded live from the web client on 2026-11-24 (messages.md).
method!(SUBSCRIPTIONS_THREAD_GET, "subscriptions.thread.get", T3);
method!(SUBSCRIPTIONS_THREAD_ADD, "subscriptions.thread.add", T3);
method!(
    SUBSCRIPTIONS_THREAD_REMOVE,
    "subscriptions.thread.remove",
    T3
);

method!(FILES_INFO, "files.info", T4);
method!(
    FILES_GET_UPLOAD_URL_EXTERNAL,
    "files.getUploadURLExternal",
    T4
);
method!(
    FILES_COMPLETE_UPLOAD_EXTERNAL,
    "files.completeUploadExternal",
    T4
);

// Search (UNVERIFIED: unverified shape, no capture; see docs/providers/slack/web-client/search.md).
method!(SEARCH_MODULES_MESSAGES, "search.modules.messages", T2);
method!(SEARCH_MODULES_FILES, "search.modules.files", T2);
method!(SEARCH_MODULES_PEOPLE, "search.modules.people", T2);
method!(SEARCH_MODULES_CHANNELS, "search.modules.channels", T2);

// Scheduled messages (UNVERIFIED: documented shape, no capture).
method!(CHAT_SCHEDULE_MESSAGE, "chat.scheduleMessage", T3);
method!(
    CHAT_SCHEDULED_MESSAGES_LIST,
    "chat.scheduledMessages.list",
    T3
);
method!(
    CHAT_DELETE_SCHEDULED_MESSAGE,
    "chat.deleteScheduledMessage",
    T3
);

// Reminders (UNVERIFIED: documented shape, no capture).
method!(REMINDERS_ADD, "reminders.add", T2);
method!(REMINDERS_LIST, "reminders.list", T2);
method!(REMINDERS_COMPLETE, "reminders.complete", T2);
method!(REMINDERS_DELETE, "reminders.delete", T2);

// People, DND and notification prefs (UNVERIFIED: documented shape, no capture; see
// docs/providers/slack/web-client/people.md). `users.prefs.set` was captured in the spike.
method!(USERS_PROFILE_GET, "users.profile.get", T4);
method!(USERS_PROFILE_SET, "users.profile.set", T3);
method!(USERS_SET_PRESENCE, "users.setPresence", T2);
method!(USERS_PREFS_GET, "users.prefs.get", T3);
method!(USERS_PREFS_SET, "users.prefs.set", T3);
method!(DND_INFO, "dnd.info", T3);
method!(DND_TEAM_INFO, "dnd.teamInfo", T2);
method!(DND_SET_SNOOZE, "dnd.setSnooze", T2);
method!(DND_END_SNOOZE, "dnd.endSnooze", T2);

// Huddles. `huddles.get` and `huddles.history` were seen in the 2026-11-13 capture
// (`tests/fixtures/spike/`); `huddles.decline` is UNVERIFIED (no capture of a decline).
method!(HUDDLES_GET, "huddles.get", T3);
method!(HUDDLES_HISTORY, "huddles.history", T3);
method!(
    /// Edge API (`edgeapi.slack.com/cache/<team>/huddles/list`), seen in the 2026-11-18 capture.
    HUDDLES_LIST,
    "huddles/list",
    T3
);
method!(HUDDLES_DECLINE, "huddles.decline", T3);
// `rooms.join` was seen in the 2026-11-13 capture and re-checked 2026-11-21.
method!(ROOMS_JOIN, "rooms.join", T3);

// Pins, bookmarks, saved items, the Threads view (UNVERIFIED: documented or assumed shape, no
// capture; see docs/providers/slack/web-client/messages.md).
method!(PINS_LIST, "pins.list", T2);
method!(PINS_ADD, "pins.add", T2);
method!(PINS_REMOVE, "pins.remove", T2);
method!(BOOKMARKS_LIST, "bookmarks.list", T3);
method!(BOOKMARKS_ADD, "bookmarks.add", T2);
method!(BOOKMARKS_REMOVE, "bookmarks.remove", T2);
method!(SAVED_LIST, "saved.list", T2);
method!(SAVED_ADD, "saved.add", T2);
method!(SAVED_DELETE, "saved.delete", T2);
method!(
    SUBSCRIPTIONS_THREAD_GET_VIEW,
    "subscriptions.thread.getView",
    T3
);
