/// What a provider supports; the UI shows a feature only when present.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub threads: bool,
    pub edit: bool,
    pub delete: bool,
    pub typing: bool,
    pub presence: bool,
    /// Other people's presence can be watched (`ChatProvider::watch_presence`).
    pub presence_watch: bool,
    /// A direct message can be opened with anyone (`ChatProvider::open_direct`).
    pub open_direct: bool,
    pub scheduled_send: bool,
    pub huddles: bool,
    /// A call window surface can be built (`ChatProvider::call_surface`).
    pub call_surface: bool,
    /// A native call join can be built (`ChatProvider::native_call_join`).
    pub native_calls: bool,
    pub custom_emoji: bool,
    pub canvases: bool,
    pub lists: bool,
    pub workflows: bool,
    pub user_groups: bool,
    pub search: bool,
    pub reminders: bool,
    pub dnd: bool,
    pub status: bool,
    pub pins: bool,
    pub bookmarks: bool,
    pub saved: bool,
    pub thread_inbox: bool,
    /// Threads can be followed and unfollowed (`ChatProvider::set_thread_followed`).
    pub thread_follow: bool,
    pub notify_prefs: bool,
    /// Members can be looked up by id (`ChatProvider::members`).
    pub member_lookup: bool,
    /// The member directory may be incomplete (the provider pages it with a
    /// cap or streams it), so a full list of people can miss some.
    pub members_partial: bool,
    /// Channels can be created, browsed, joined, left, archived, renamed,
    /// given a topic and a description, and people invited to them
    /// (`ChatProvider::create_channel` and the methods after it).
    pub channel_admin: bool,
    /// A conversation can be fetched by id (`ChatProvider::conversation`), for
    /// a live change to one the app does not hold yet.
    pub conversation_lookup: bool,
    /// Stars are kept by the provider too (`ChatProvider::set_conversation_starred`);
    /// Parla's own starred list is sent there as it changes.
    pub conversation_stars: bool,
    /// Files can be uploaded and their metadata and bytes read
    /// (`ChatProvider::upload`, file downloads).
    pub files: bool,
    /// Conversations can be marked read (`ChatProvider::mark_read`).
    pub read_state: bool,
    /// Messages can be searched (`ChatProvider::search_messages`).
    pub search_messages: bool,
    /// Files can be searched (`ChatProvider::search_files`).
    pub search_files: bool,
}
