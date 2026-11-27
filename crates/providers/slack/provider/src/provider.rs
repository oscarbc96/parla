use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use domain::{Bookmark, BookmarkId, InboxThread, Pin, SavedItem};
use domain::{
    Bootstrap, CallSurface, Capabilities, ChatProvider, ClientMsgId, ContentParser,
    ConversationCounts, ConversationId, ConversationKind, CustomEmoji, EmojiRef, ExtCodec,
    HistoryPage, Huddle, HuddleId, Message, MessageId, MessageTs, NativeCallJoin, Page,
    ProviderError, ProviderEvent, Reminder, ReminderId, RemoteUrl, ScheduleId, ScheduledMessage,
    SecretText, SurfaceCookie, Upload, UserId,
};
use domain::{ConversationPrefs, DndStatus, PresenceSetting, Profile, Status};
use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use settings::WorkspaceKey;
use slack_api::{
    ApiError, ClientConfig, PresenceMode, Priority, SessionCookie, WebClient, XoxcToken,
};
use slack_auth::WorkspaceCredentials;
use slack_realtime::conn::{ConnConfig, ConnEvent, RealtimeHandle, spawn};
use slack_realtime::event::{NOTIFY_PREF_NAMES, Outgoing, SlackEvent};
use slack_realtime::url::ClientBuilt;
use tokio::sync::{broadcast, mpsc};
use url::Url;

use crate::channel_admin::AdminOp;
use crate::ext;
use crate::files::map_file;
use crate::huddles::map_room;
use crate::map::{
    group_dm_fallback_open, level_value, map_bookmark, map_conversation, map_counts, map_dnd,
    map_error, map_events_as, map_inbox, map_message_as, map_notify_prefs, map_pin, map_profile,
    map_saved, map_user, ts_opt, user_id,
};

/// Upper bound on `chat.scheduledMessages.list` pages fetched in all.
const MAX_SCHEDULED_PAGES: usize = 20;

/// Most users one `dnd_users` call answers for; more is refused, not silently cut.
const DND_USERS_MAX: usize = 500;

/// Used when `client.shouldReload` gives no build version.
const FALLBACK_AGENT_VERSION: &str = "0";
/// Upper bound on `users.list` pages fetched in all.
const USERS_MAX_PAGES: usize = 50;
/// Pages bootstrap waits for; within the background burst, so no pacing delay.
const USERS_BOOT_PAGES: usize = 8;
const EVENT_BUFFER: usize = 4096;
/// Ids remembered to drop a message the live socket delivers twice (catch-up does not use it).
const EMITTED_CAP: usize = 8192;
/// Messages with files remembered for refresh.
const FILE_MSGS_CAP: usize = 256;
/// Bot ids whose message-carried name was already announced.
const BOTS_SEEN_CAP: usize = 2048;
/// `users.info` and `bots.info` calls in flight per lookup batch.
const MEMBER_LOOKUP_CONCURRENCY: usize = 4;
const UPLOAD_SHARE_TRIES: usize = 5;
const UPLOAD_SHARE_DELAY: std::time::Duration = std::time::Duration::from_millis(400);

/// Adjusts the websocket connection config.
pub type ConnTune = Arc<dyn Fn(&mut ConnConfig) + Send + Sync>;

/// Websocket settings; the default is the production host and timings.
#[derive(Clone, Default)]
pub struct WsOptions {
    /// Connect base URL; `None` is the production host.
    pub base: Option<Url>,
    /// Adjusts the connection config (timings, headers).
    pub conn: Option<ConnTune>,
}

#[derive(Default)]
pub(crate) struct Seen {
    emitted: HashSet<(String, String)>,
    order: VecDeque<(String, String)>,
}

/// Cached conversation facts needed to name a huddle without a network call.
#[derive(Clone)]
pub(crate) struct CachedConversation {
    pub kind: domain::ConversationKind,
    pub name: Arc<str>,
    pub peer: Option<String>,
    pub member_ids: Vec<String>,
    pub participants: Vec<String>,
}

pub(crate) struct Shared {
    pub client: WebClient,
    pub(crate) self_user: String,
    origin: String,
    tx: broadcast::Sender<ProviderEvent>,
    first_rx: Mutex<Option<broadcast::Receiver<ProviderEvent>>>,
    realtime: Mutex<Option<RealtimeHandle>>,
    expired: AtomicBool,
    pub seen: Mutex<Seen>,
    presence_ids: Mutex<Vec<String>>,
    presence: bool,
    /// Recently emitted messages that carry files, to rebuild them on file events.
    file_msgs: Mutex<VecDeque<Message>>,
    /// A notification-prefs refetch is queued; a burst of `pref_change` frames shares one.
    prefs_pending: AtomicBool,
    /// File ids with a refresh queued; a burst of frames for one file shares one.
    files_pending: Mutex<HashSet<String>>,
    /// One file refresh at a time, so an older `files.info` never lands after a newer one.
    files_read: tokio::sync::Mutex<()>,
    /// Serialises the notification-prefs read-modify-write so two writes never lose one.
    prefs_write: tokio::sync::Mutex<()>,
    /// One notification-prefs refetch at a time.
    prefs_read: tokio::sync::Mutex<()>,
    /// Bot ids already announced as members from a message's own name.
    bots_seen: Mutex<HashSet<String>>,
    /// Conversation facts cached at bootstrap (and on open) so call titles need
    /// no network call.
    pub(crate) conversations: Mutex<HashMap<String, CachedConversation>>,
    /// Display name by user id or group handle, cached at bootstrap.
    pub(crate) member_names: Mutex<HashMap<String, Arc<str>>>,
}

impl Shared {
    /// Remembers the facts a call title needs, so a conversation opened after
    /// bootstrap is named too.
    pub(crate) fn cache_conversation(&self, conv: &domain::Conversation) {
        self.conversations.lock().unwrap().insert(
            conv.id.0.to_string(),
            CachedConversation {
                kind: conv.kind,
                name: conv.name.clone(),
                peer: conv.peer.as_ref().map(|u| u.0.to_string()),
                member_ids: conv.member_ids.iter().map(|u| u.0.to_string()).collect(),
                participants: conv.participants.iter().map(|h| h.to_string()).collect(),
            },
        );
    }

    /// Remembers a message with files (newest wins) for later refreshes.
    pub fn index_files(&self, m: &Message) {
        if m.files.is_empty() {
            return;
        }
        let mut q = self.file_msgs.lock().unwrap();
        q.retain(|o| o.id != m.id);
        q.push_back(m.clone());
        while q.len() > FILE_MSGS_CAP {
            q.pop_front();
        }
    }

    /// A bot message names its author itself (`bot_profile.name`, `username`):
    /// announce that once per bot as a member, so no lookup is needed.
    pub fn note_bot(&self, m: &slack_api::Message) {
        let Some((id, name)) = m.bot_identity() else {
            return;
        };
        {
            let mut seen = self.bots_seen.lock().unwrap();
            if seen.len() >= BOTS_SEEN_CAP {
                seen.clear();
            }
            if !seen.insert(id.to_string()) {
                return;
            }
        }
        let icon = crate::map::avatar_ref([
            m.bot_profile
                .as_ref()
                .and_then(|b| b.icons.image_72.as_deref()),
            m.bot_profile
                .as_ref()
                .and_then(|b| b.icons.image_48.as_deref()),
        ]);
        self.emit(ProviderEvent::MemberUpdated(bot_member(id, name, icon)));
    }

    fn messages_with_file(&self, id: &str) -> Vec<Message> {
        let q = self.file_msgs.lock().unwrap();
        q.iter()
            .filter(|m| m.files.iter().any(|f| &*f.id.0 == id))
            .cloned()
            .collect()
    }

    /// `files.info` for a changed file, then a full `MessageUpdated` per message holding it.
    async fn refresh_file(&self, file_id: &str) {
        let _one_at_a_time = self.files_read.lock().await;
        // Cleared before the read so a change that lands during it queues another.
        self.files_pending.lock().unwrap().remove(file_id);
        if self.messages_with_file(file_id).is_empty() {
            return;
        }
        let info = match self.client.files_info(file_id).await {
            Ok(f) => f,
            Err(e) => {
                if e.is_auth() {
                    self.expire();
                }
                return;
            }
        };
        let fresh = map_file(&info);
        for mut m in self.messages_with_file(file_id) {
            m.files = m
                .files
                .iter()
                .map(|f| {
                    if f.id == fresh.id {
                        fresh.clone()
                    } else {
                        f.clone()
                    }
                })
                .collect();
            self.index_files(&m);
            self.emit(ProviderEvent::MessageUpdated(m));
        }
    }

    /// Pages `users.list`: the first [`USERS_BOOT_PAGES`] go to `head`, later
    /// pages are emitted as member updates.
    async fn load_users(
        self: Arc<Self>,
        head: futures::channel::oneshot::Sender<Result<Vec<slack_api::User>, ApiError>>,
    ) {
        let mut head = Some(head);
        let mut first = Vec::new();
        let mut n = 0;
        let mut pages = std::pin::pin!(
            self.client
                .users_list(Priority::Background)
                .take(USERS_MAX_PAGES)
        );
        while let Some(p) = pages.next().await {
            n += 1;
            match (p, head.is_some()) {
                (Ok(v), true) => first.extend(v),
                (Ok(v), false) => {
                    for u in v {
                        let m = map_user(u);
                        index_member_names(&mut self.member_names.lock().unwrap(), &m);
                        self.emit(ProviderEvent::MemberUpdated(m));
                    }
                }
                (Err(e), true) => {
                    if let Some(h) = head.take() {
                        let _ = h.send(Err(e));
                    }
                    return;
                }
                (Err(_), false) => return,
            }
            if n == USERS_BOOT_PAGES
                && let Some(h) = head.take()
            {
                let _ = h.send(Ok(std::mem::take(&mut first)));
            }
        }
        if let Some(h) = head.take() {
            let _ = h.send(Ok(first));
        }
    }

    /// Re-reads the notification prefs after a `pref_change` frame and emits them. Best effort:
    /// one attempt at background priority per read, one read at a time. A frame that lands
    /// during a read queues one more refresh (the pending flag is re-checked after it).
    async fn refresh_notify_prefs(&self) {
        let _one_at_a_time = self.prefs_read.lock().await;
        if !self.prefs_pending.load(Ordering::SeqCst) {
            // The read that held the lock already covered this change.
            return;
        }
        loop {
            // Cleared before the read so a change that lands during it queues another.
            self.prefs_pending.store(false, Ordering::SeqCst);
            match self.client.prefs_get_once().await {
                Ok(raw) => self.emit(ProviderEvent::NotifyPrefsChanged(map_notify_prefs(&raw))),
                Err(e) => {
                    let _ = self.fail(&e);
                }
            }
            if !self.prefs_pending.load(Ordering::SeqCst) {
                break;
            }
        }
    }

    fn emit(&self, e: ProviderEvent) {
        let _ = self.tx.send(e);
    }

    /// Emits a message once; false when it was already emitted.
    pub fn emit_posted(&self, m: Message) -> bool {
        let key = (m.id.conversation.0.to_string(), m.id.ts.0.to_string());
        {
            let mut s = self.seen.lock().unwrap();
            if !s.emitted.insert(key.clone()) {
                return false;
            }
            s.order.push_back(key.clone());
            if s.order.len() > EMITTED_CAP
                && let Some(old) = s.order.pop_front()
            {
                s.emitted.remove(&old);
            }
        }
        self.index_files(&m);
        self.emit(ProviderEvent::MessagePosted(m));
        true
    }

    /// The session is gone: tell the app once and stop the socket for good.
    pub fn expire(&self) {
        if self.expired.swap(true, Ordering::SeqCst) {
            return;
        }
        self.emit(ProviderEvent::SessionExpired);
        self.stop_realtime();
    }

    /// Ends the realtime backoff wait so the next attempt starts now.
    fn retry_realtime(&self) {
        if let Some(h) = self.realtime.lock().unwrap().as_ref() {
            h.retry_now();
        }
    }

    fn stop_realtime(&self) {
        if let Some(h) = self.realtime.lock().unwrap().take() {
            h.shutdown();
        }
    }

    /// Keeps the socket handle unless the session already ended (or the provider was
    /// dropped), checked under the same lock `stop_realtime` takes so a handle stored
    /// just as `expire` runs is still shut down.
    fn store_realtime(&self, handle: RealtimeHandle) {
        let mut slot = self.realtime.lock().unwrap();
        if self.expired.load(Ordering::SeqCst) {
            drop(slot);
            handle.shutdown();
            return;
        }
        *slot = Some(handle);
    }

    pub(crate) fn fail(&self, e: &ApiError) -> ProviderError {
        if e.is_auth() {
            self.expire();
        }
        map_error(e)
    }

    fn send_presence(&self) {
        if !self.presence {
            return;
        }
        let ids = self.presence_ids.lock().unwrap().clone();
        if ids.is_empty() {
            return;
        }
        if let Some(h) = self.realtime.lock().unwrap().as_ref() {
            h.send(Outgoing::PresenceSub {
                id: h.next_id(),
                ids: ids.clone(),
            });
            h.send(Outgoing::PresenceQuery {
                id: h.next_id(),
                ids,
            });
        }
    }
}

/// Maps an API error to this provider's error, expiring the session on an auth failure.
/// Every `ChatProvider` method ends in `.or_fail(&self.sh)?`.
pub(crate) trait OrFail<T> {
    fn or_fail(self, sh: &Shared) -> Result<T, ProviderError>;
}

impl<T> OrFail<T> for Result<T, ApiError> {
    fn or_fail(self, sh: &Shared) -> Result<T, ProviderError> {
        self.map_err(|e| sh.fail(&e))
    }
}

pub struct SlackProvider {
    ws: WorkspaceKey,
    caps: Capabilities,
    pub(crate) sh: Arc<Shared>,
}

impl Drop for SlackProvider {
    /// Sign-out or workspace removal drops the provider: stop the socket, its pings and
    /// reconnects, which would otherwise run (through the pump's `Arc<Shared>`) until exit.
    fn drop(&mut self) {
        self.sh.expired.store(true, Ordering::SeqCst);
        self.sh.stop_realtime();
    }
}

fn dup_secrets(c: &WorkspaceCredentials) -> (XoxcToken, SessionCookie) {
    (c.token.duplicate(), c.cookie.duplicate())
}

impl SlackProvider {
    /// Starts the session socket in the background; production websocket settings.
    pub fn start(
        creds: WorkspaceCredentials,
        cfg: ClientConfig,
        rt: &tokio::runtime::Handle,
    ) -> SlackProvider {
        Self::start_with(creds, cfg, WsOptions::default(), rt)
    }

    pub fn start_with(
        creds: WorkspaceCredentials,
        cfg: ClientConfig,
        ws: WsOptions,
        rt: &tokio::runtime::Handle,
    ) -> SlackProvider {
        let (tx, first_rx) = broadcast::channel(EVENT_BUFFER);
        let caps = Capabilities {
            threads: true,
            edit: true,
            delete: true,
            typing: true,
            presence: true,
            presence_watch: true,
            open_direct: true,
            // `users.list` stops at USERS_MAX_PAGES and streams after boot.
            members_partial: true,
            scheduled_send: true,
            // Confirmed by the huddle captures (docs/providers/slack/web-client/huddles.md).
            huddles: true,
            call_surface: true,
            native_calls: true,
            custom_emoji: true,
            canvases: true,
            lists: true,
            workflows: true,
            user_groups: true,
            search: true,
            dnd: true,
            status: true,
            notify_prefs: false,
            // `pins`, `bookmarks`, `saved` and `thread_inbox` stay off until a capture confirms
            // the routes (docs/providers/slack/web-client/messages.md).
            member_lookup: true,
            // Seen in the 2026-11-24 web-client capture (conversations.md "Channel admin").
            channel_admin: true,
            // `conversations.info`, for a live change to a channel not held yet.
            conversation_lookup: true,
            conversation_stars: true,
            // Recorded live on 2026-11-24 (messages.md "Thread follow").
            thread_follow: true,
            files: true,
            read_state: true,
            search_messages: true,
            search_files: true,
            ..Capabilities::default()
        };
        let recorder = slack_api::recorder_from_env();
        let (token, cookie) = dup_secrets(&creds);
        let cfg_retry = cfg.clone();
        let client = WebClient::new(cfg, token, cookie);
        let sh = Arc::new(Shared {
            client: match (client, &recorder) {
                (Ok(c), Some(r)) => c.with_recorder(r.clone()),
                (Ok(c), None) => c,
                // Cannot happen with a validated token and cookie; retry the
                // build once more, which panics if the HTTP stack is unusable.
                (Err(_), _) => unreachable_client(&creds, cfg_retry, &recorder),
            },
            self_user: creds.user_id.clone(),
            origin: creds.url.as_str().trim_end_matches('/').to_string(),
            tx,
            first_rx: Mutex::new(Some(first_rx)),
            realtime: Mutex::new(None),
            expired: AtomicBool::new(false),
            seen: Mutex::new(Seen::default()),
            presence_ids: Mutex::new(Vec::new()),
            presence: caps.presence,
            file_msgs: Mutex::new(VecDeque::new()),
            files_pending: Mutex::new(HashSet::new()),
            files_read: tokio::sync::Mutex::new(()),
            prefs_pending: AtomicBool::new(false),
            prefs_write: tokio::sync::Mutex::new(()),
            prefs_read: tokio::sync::Mutex::new(()),
            bots_seen: Mutex::new(HashSet::new()),
            conversations: Mutex::new(HashMap::new()),
            member_names: Mutex::new(HashMap::new()),
        });
        let provider = SlackProvider {
            ws: creds.workspace.clone(),
            caps,
            sh: sh.clone(),
        };
        let team_id = creds.workspace.id.0.clone();
        let handle = rt.clone();
        rt.spawn(async move {
            run(sh, creds, team_id, ws, recorder, handle).await;
        });
        provider
    }
}

impl SlackProvider {
    /// Refreshes the cached name, icon and organisation in the account index
    /// once, in the background, over this provider's own client (same pool and
    /// rate limiter). One attempt; a failure is ignored.
    pub(crate) fn spawn_account_refresh(
        &self,
        accounts_path: std::path::PathBuf,
        rt: &tokio::runtime::Handle,
    ) {
        let sh = self.sh.clone();
        let ws = self.ws.clone();
        rt.spawn(async move {
            let _ = crate::factory::refresh_account(&accounts_path, &ws, &sh.client).await;
        });
    }
}

fn unreachable_client(
    creds: &WorkspaceCredentials,
    cfg: ClientConfig,
    recorder: &Option<Arc<redact::record::Recorder>>,
) -> WebClient {
    // `WebClient::new` only fails when the HTTP stack cannot be built.
    let (token, cookie) = dup_secrets(creds);
    let client = WebClient::new(cfg, token, cookie).expect("http client");
    match recorder {
        Some(r) => client.with_recorder(r.clone()),
        None => client,
    }
}

/// Background task: resolve the build version, open the socket, pump events.
async fn run(
    sh: Arc<Shared>,
    creds: WorkspaceCredentials,
    team_id: String,
    ws: WsOptions,
    recorder: Option<Arc<redact::record::Recorder>>,
    rt: tokio::runtime::Handle,
) {
    let version = match sh.client.build_version().await {
        Ok(Some(v)) => v,
        Ok(None) => FALLBACK_AGENT_VERSION.to_string(),
        Err(e) if e.is_auth() => {
            sh.expire();
            return;
        }
        Err(_) => FALLBACK_AGENT_VERSION.to_string(),
    };
    let (token, cookie) = dup_secrets(&creds);
    let mut built = ClientBuilt::new(team_id, token, sh.client.clone(), version);
    if let Some(b) = ws.base {
        built.base = b;
    }
    let mut conn = ConnConfig::new(cookie);
    conn.recorder = recorder;
    if let Some(f) = &ws.conn {
        f(&mut conn);
    }
    drop(creds);
    let (handle, rx) = spawn(conn, Arc::new(built), &rt);
    sh.store_realtime(handle);
    pump(sh, rx).await;
}

async fn pump(sh: Arc<Shared>, mut rx: mpsc::Receiver<ConnEvent>) {
    while let Some(ev) = rx.recv().await {
        match ev {
            ConnEvent::Connected { reconnect } => {
                sh.emit(ProviderEvent::Connected { reconnect });
                sh.send_presence();
                // Catch-up after a reconnect is planned by `domain::sync`, driven
                // by the `Connected { reconnect: true }` event.
            }
            ConnEvent::Event(e) => {
                if let SlackEvent::Message { message, .. } = &e {
                    sh.note_bot(message);
                }
                if let SlackEvent::FileShared { file_id, .. } | SlackEvent::FileChange { file_id } =
                    &e
                {
                    if sh.files_pending.lock().unwrap().insert(file_id.clone()) {
                        let (sh, id) = (sh.clone(), file_id.clone());
                        tokio::spawn(async move { sh.refresh_file(&id).await });
                    }
                    continue;
                }
                if let SlackEvent::PrefChange { name, .. } = &e {
                    if NOTIFY_PREF_NAMES.contains(&name.as_str())
                        && !sh.prefs_pending.swap(true, Ordering::SeqCst)
                    {
                        let sh = sh.clone();
                        tokio::spawn(async move { sh.refresh_notify_prefs().await });
                    }
                    continue;
                }
                // The self DND frame may omit the user: it is the signed-in one.
                let e = match e {
                    SlackEvent::DndUpdated { user, dnd } if user.is_empty() => {
                        SlackEvent::DndUpdated {
                            user: sh.self_user.clone(),
                            dnd,
                        }
                    }
                    e => e,
                };
                for pe in map_events_as(e, Some(sh.self_user.as_str())) {
                    match pe {
                        ProviderEvent::MessagePosted(m) => {
                            sh.emit_posted(m);
                        }
                        ProviderEvent::MessageUpdated(m) => {
                            sh.index_files(&m);
                            sh.emit(ProviderEvent::MessageUpdated(m));
                        }
                        other => sh.emit(other),
                    }
                }
            }
            ConnEvent::Disconnected { reason } => {
                // Cause, fresh or resumed URL and uptime; never a URL, token or id.
                log::info!("realtime lost: {reason}");
                sh.emit(ProviderEvent::Disconnected)
            }
            ConnEvent::AuthExpired => {
                sh.expire();
                return;
            }
        }
    }
}

/// A secure, http-only session cookie for the call surface on the workspace host.
fn surface_cookie(name: &str, value: &str) -> SurfaceCookie {
    SurfaceCookie {
        name: Arc::from(name),
        value: SecretText::new(value.to_string()),
        domain: Arc::from(".slack.com"),
        path: Arc::from("/"),
        secure: true,
        http_only: true,
    }
}

/// Adds a member's names to the lookup: the id maps to the display name, else the real
/// name, else the handle (never the raw id); the handle maps to the display or real name,
/// else to itself.
fn index_member_names(names: &mut HashMap<String, Arc<str>>, m: &domain::Member) {
    if let Some(display) = [&m.display_name, &m.real_name, &m.handle]
        .into_iter()
        .find(|n| !n.is_empty())
    {
        names.insert(m.id.0.to_string(), display.clone());
    }
    if !m.handle.is_empty() {
        let hd = [&m.display_name, &m.real_name]
            .into_iter()
            .find(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| Arc::from(m.handle.to_string()));
        names.insert(m.handle.to_string(), hd);
    }
}

fn names_of(members: &[domain::Member]) -> HashMap<String, Arc<str>> {
    members
        .iter()
        .filter_map(|m| {
            // Display name, else real name, else handle; never the raw id.
            [&m.display_name, &m.real_name, &m.handle]
                .into_iter()
                .find(|n| !n.is_empty())
                .map(|n| (m.id.0.to_string(), n.clone()))
        })
        .collect()
}

/// Custom emoji with aliases followed to their image; an alias of a standard
/// emoji (or a broken chain) has no image and is dropped.
fn resolve_emoji(raw: &std::collections::BTreeMap<String, String>) -> Vec<CustomEmoji> {
    const MAX_ALIAS_DEPTH: usize = 5;
    raw.iter()
        .filter_map(|(name, value)| {
            let mut v = value.as_str();
            for _ in 0..MAX_ALIAS_DEPTH {
                match v.strip_prefix("alias:") {
                    Some(target) => v = raw.get(target)?.as_str(),
                    None => {
                        return Some(CustomEmoji {
                            shortcode: Arc::from(name.as_str()),
                            image: domain::ImageRef::Provider(RemoteUrl(Arc::from(v))),
                        });
                    }
                }
            }
            None
        })
        .collect()
}

/// A bot as a member: it has no profile, only the name it posts under.
fn bot_member(id: &str, name: &str, icon: Option<domain::ImageRef>) -> domain::Member {
    domain::Member {
        id: user_id(id),
        display_name: Arc::from(name),
        real_name: Arc::from(name),
        handle: Arc::from(""),
        is_bot: true,
        deleted: false,
        status: None,
        title: None,
        tz: None,
        avatar: domain::Avatar::new(icon.clone(), icon),
        ext: domain::ProviderExt::none(),
    }
}

impl SlackProvider {
    /// `users.info` per user id and `bots.info` per bot id (`B...`), at most
    /// [`MEMBER_LOOKUP_CONCURRENCY`] in flight; the shared limiter paces them.
    /// Ids that fail to resolve are left out; when every id failed the first
    /// error is returned so the caller can tell transient from permanent.
    async fn member_one(&self, id: String) -> Result<domain::Member, ApiError> {
        if id.starts_with('B') {
            self.sh.client.bots_info(&id).await.map(|b| {
                let icon = crate::map::avatar_ref([
                    b.icons.image_72.as_deref(),
                    b.icons.image_48.as_deref(),
                ]);
                bot_member(&id, &b.name, icon)
            })
        } else {
            self.sh.client.users_info(&id).await.map(map_user)
        }
    }

    async fn do_members(&self, ids: &[UserId]) -> Result<Vec<domain::Member>, ProviderError> {
        let lookups: Vec<_> = ids
            .iter()
            .map(|u| u.0.to_string())
            .map(|id| Box::pin(self.member_one(id)) as futures::future::BoxFuture<'_, _>)
            .collect();
        let results: Vec<_> = futures::stream::iter(lookups)
            .buffered(MEMBER_LOOKUP_CONCURRENCY)
            .collect()
            .await;
        let mut found = Vec::new();
        let mut first_err = None;
        for r in results {
            match r {
                Ok(m) => found.push(m),
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        match (found.is_empty(), first_err) {
            (true, Some(e)) => Err(self.sh.fail(&e)),
            _ => Ok(found),
        }
    }

    fn page(&self, c: &ConversationId, p: slack_api::HistoryPage) -> HistoryPage {
        let me = Some(self.sh.self_user.as_str());
        p.messages.iter().for_each(|m| self.sh.note_bot(m));
        let messages: Vec<Message> = p
            .messages
            .into_iter()
            .map(|m| map_message_as(&c.0, m, me))
            .collect();
        messages.iter().for_each(|m| self.sh.index_files(m));
        HistoryPage {
            messages,
            has_more: p.has_more,
        }
    }

    /// External upload flow, then the message that shares the files. The message
    /// comes from `shares` in `files.info`; it can lag the upload briefly.
    async fn share_files(
        &self,
        c: &ConversationId,
        thread: Option<MessageTs>,
        files: &[Upload],
        text: Option<String>,
        id: ClientMsgId,
    ) -> Result<Message, ProviderError> {
        let paths: Vec<(&std::path::Path, &str, Option<&str>)> = files
            .iter()
            .map(|u| (u.path.as_path(), &*u.name, u.filetype.as_deref()))
            .collect();
        let done = self
            .sh
            .client
            .upload_files(
                &c.0,
                thread.as_ref().map(|t| &*t.0),
                &paths,
                text.as_deref(),
            )
            .await
            .or_fail(&self.sh)?;
        let mut infos = Vec::new();
        let mut ts = None;
        for attempt in 0..UPLOAD_SHARE_TRIES {
            infos.clear();
            for f in &done {
                infos.push(self.sh.client.files_info(&f.id).await.or_fail(&self.sh)?);
            }
            ts = infos.iter().find_map(|f| f.share_ts(&c.0));
            if ts.is_some() {
                break;
            }
            if attempt + 1 < UPLOAD_SHARE_TRIES {
                tokio::time::sleep(UPLOAD_SHARE_DELAY).await;
            }
        }
        let ts = ts.ok_or_else(|| {
            ProviderError::Localized(i18n::LocalizedText::new(
                "slack",
                "error-upload-not-visible",
            ))
        })?;
        let raw = slack_api::Message {
            r#type: "message".into(),
            ts,
            user: Some(self.sh.self_user.clone()),
            text: text.unwrap_or_default(),
            thread_ts: thread.map(|t| t.0.to_string()),
            ..Default::default()
        };
        let mut m = map_message_as(&c.0, raw, Some(&self.sh.self_user));
        m.files = infos.iter().map(map_file).collect();
        m.client_msg_id = Some(id);
        self.sh.index_files(&m);
        Ok(m)
    }

    async fn post(
        &self,
        c: &ConversationId,
        text: &str,
        thread: Option<&MessageTs>,
        broadcast: bool,
        id: ClientMsgId,
    ) -> Result<Message, ProviderError> {
        let started = std::time::Instant::now();
        let posted = match thread {
            Some(t) => {
                self.sh
                    .client
                    .post_reply(&c.0, &t.0, text, broadcast, &id.0)
                    .await
            }
            None => self.sh.client.post_message(&c.0, text, None, &id.0).await,
        }
        .inspect(|_| log::debug!("send answered in {} ms", started.elapsed().as_millis()))
        .or_fail(&self.sh)?;
        let mut m = posted.message;
        if m.ts.is_empty() {
            m.ts = posted.ts;
        }
        let channel = if posted.channel.is_empty() {
            &*c.0
        } else {
            posted.channel.as_str()
        };
        let mut d = map_message_as(channel, m, Some(&self.sh.self_user));
        d.client_msg_id = Some(id);
        Ok(d)
    }
}

impl SlackProvider {
    /// `not_found` on complete or delete means the reminder is already gone: a not-allowed
    /// error, which the domain settles by reloading the list rather than rolling back.
    fn reminder_fail(&self, e: &ApiError) -> ProviderError {
        match e {
            ApiError::Slack { error, .. } if error == "not_found" => {
                ProviderError::NotAllowed(error.clone())
            }
            _ => self.sh.fail(e),
        }
    }
}

impl SlackProvider {
    /// Builds the huddle's display name without a network call, from cached
    /// conversation facts.
    pub(crate) fn call_title(&self, c: &domain::ConversationId) -> Arc<str> {
        let convs = self.sh.conversations.lock().unwrap();
        let members = self.sh.member_names.lock().unwrap();
        call_title_from_maps(&convs, &members, c)
    }
}

pub(crate) fn call_title_from_maps(
    convs: &HashMap<String, CachedConversation>,
    members: &HashMap<String, Arc<str>>,
    c: &domain::ConversationId,
) -> Arc<str> {
    let Some(conv) = convs.get(c.0.as_ref()) else {
        return Arc::from("");
    };
    match conv.kind {
        domain::ConversationKind::Channel | domain::ConversationKind::PrivateChannel => {
            if conv.name.is_empty() {
                Arc::from("")
            } else {
                Arc::from(format!("#{}", conv.name))
            }
        }
        domain::ConversationKind::Direct => {
            if !conv.name.is_empty() {
                conv.name.clone()
            } else {
                conv.peer
                    .as_ref()
                    .and_then(|p| members.get(p).cloned())
                    .unwrap_or_default()
            }
        }
        domain::ConversationKind::Group => {
            let mut names: Vec<Arc<str>> = Vec::new();
            for h in &conv.participants {
                if let Some(d) = members.get(h) {
                    names.push(d.clone());
                } else {
                    names.push(Arc::from(h.as_str()));
                }
            }
            if names.is_empty() {
                for id in &conv.member_ids {
                    if let Some(d) = members.get(id) {
                        names.push(d.clone());
                    }
                }
            }
            Arc::from(names.join(", "))
        }
    }
}

#[cfg(test)]
mod call_title_tests {
    use super::*;
    use domain::{ConversationId, ConversationKind};

    fn conv(
        kind: ConversationKind,
        name: &str,
        peer: Option<&str>,
        member_ids: &[&str],
        participants: &[&str],
    ) -> CachedConversation {
        CachedConversation {
            kind,
            name: Arc::from(name),
            peer: peer.map(str::to_owned),
            member_ids: member_ids.iter().map(|s| (*s).to_owned()).collect(),
            participants: participants.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn channel_gives_hash_name() {
        let mut convs = HashMap::new();
        convs.insert(
            "C1".to_owned(),
            conv(ConversationKind::Channel, "general", None, &[], &[]),
        );
        let members = HashMap::new();
        assert_eq!(
            &*call_title_from_maps(&convs, &members, &ConversationId(Arc::from("C1"))),
            "#general"
        );
    }

    #[test]
    fn dm_resolved_through_peer() {
        let mut convs = HashMap::new();
        convs.insert(
            "D1".to_owned(),
            conv(ConversationKind::Direct, "", Some("U2"), &[], &[]),
        );
        let mut members = HashMap::new();
        members.insert("U2".to_owned(), Arc::from("Bob B"));
        assert_eq!(
            &*call_title_from_maps(&convs, &members, &ConversationId(Arc::from("D1"))),
            "Bob B"
        );
    }

    #[test]
    fn group_from_participants() {
        let mut convs = HashMap::new();
        convs.insert(
            "G1".to_owned(),
            conv(
                ConversationKind::Group,
                "mpdm-a--b-1",
                None,
                &[],
                &["alice", "bob"],
            ),
        );
        let mut members = HashMap::new();
        members.insert("alice".to_owned(), Arc::from("Alice A"));
        members.insert("bob".to_owned(), Arc::from("Bob B"));
        assert_eq!(
            &*call_title_from_maps(&convs, &members, &ConversationId(Arc::from("G1"))),
            "Alice A, Bob B"
        );
    }

    #[test]
    fn unknown_conversation_gives_empty() {
        let convs = HashMap::new();
        let members = HashMap::new();
        assert_eq!(
            &*call_title_from_maps(&convs, &members, &ConversationId(Arc::from("C404"))),
            ""
        );
    }

    #[test]
    fn dm_peer_not_cached_gives_empty() {
        let mut convs = HashMap::new();
        convs.insert(
            "D1".to_owned(),
            conv(ConversationKind::Direct, "", Some("U2"), &[], &[]),
        );
        let members = HashMap::new();
        assert_eq!(
            &*call_title_from_maps(&convs, &members, &ConversationId(Arc::from("D1"))),
            ""
        );
    }
}

impl ChatProvider for SlackProvider {
    fn workspace(&self) -> &WorkspaceKey {
        &self.ws
    }

    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn bootstrap(&self) -> BoxFuture<'_, Result<Bootstrap, ProviderError>> {
        Box::pin(async move {
            let client = &self.sh.client;
            // The directory is paged and rate limited (background tier), so a
            // big workspace takes minutes. Bootstrap waits only for the first
            // pages; the rest arrive later as member updates.
            let (head_tx, head_rx) = futures::channel::oneshot::channel();
            tokio::spawn(self.sh.clone().load_users(head_tx));
            let users = async {
                head_rx
                    .await
                    .unwrap_or_else(|_| Err(ApiError::Transport("user list task ended".into())))
            };
            let (boot, counts, users) = futures::join!(client.user_boot(), client.counts(), users);
            let boot = boot.or_fail(&self.sh)?;
            let counts = counts.or_fail(&self.sh)?;
            let users = users.or_fail(&self.sh)?;
            let members: Vec<_> = users.into_iter().map(map_user).collect();
            let names = names_of(&members);
            let by_id: HashMap<&str, &slack_api::ChannelCount> = counts
                .channels
                .iter()
                .chain(&counts.mpims)
                .chain(&counts.ims)
                .map(|c| (c.id.as_str(), c))
                .collect();
            let lookup = |u: &str| names.get(u).cloned();
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            // `client.userBoot` sometimes carries no conversations (or only a
            // hint of them); the documented list endpoint is the fallback.
            let listed = if boot.channels.is_empty() {
                let mut out = Vec::new();
                let mut pages = std::pin::pin!(client.users_conversations(
                    "public_channel,private_channel,mpim,im",
                    Priority::Visible
                ));
                while let Some(p) = pages.next().await {
                    out.extend(p.or_fail(&self.sh)?);
                }
                // Boot may still carry the DMs; do not list them twice.
                let dms: HashSet<&str> = boot.ims.iter().map(|i| i.id.as_str()).collect();
                out.retain(|c| !dms.contains(c.id.as_str()));
                out
            } else {
                Vec::new()
            };
            let listed_count = listed.len();
            let mut conversations: Vec<_> = boot
                .channels
                .iter()
                .chain(&boot.ims)
                .chain(&listed)
                .map(|c| {
                    let count = by_id.get(c.id.as_str()).copied();
                    let mut conv = map_conversation(c, count, &lookup);
                    if let Some(open) = group_dm_fallback_open(c, count, now_secs) {
                        conv.is_open = open;
                    }
                    conv
                })
                .collect();
            // `client.userBoot` lists only the user's own conversations and
            // omits `is_member`, so its channels would otherwise be hidden.
            for c in &mut conversations[..boot.channels.len()] {
                c.is_member = true;
            }
            // users.conversations omits is_member; every entry is a membership.
            let from = conversations.len() - listed_count;
            for c in &mut conversations[from..] {
                c.is_member = true;
            }
            // Cache conversation facts and member display names so huddle titles
            // can be built without a network call.
            {
                let mut mn = self.sh.member_names.lock().unwrap();
                for m in &members {
                    index_member_names(&mut mn, m);
                }
            }
            {
                for conv in &conversations {
                    self.sh.cache_conversation(conv);
                }
            }

            Ok(Bootstrap {
                self_user: domain::UserId(Arc::from(boot.self_user.id.as_str())),
                workspace_name: Arc::from(boot.team.name.as_str()),
                conversations,
                members,
            })
        })
    }

    fn history(
        &self,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: u32,
    ) -> BoxFuture<'_, Result<HistoryPage, ProviderError>> {
        let c = c.clone();
        Box::pin(async move {
            let p = self
                .sh
                .client
                .conversations_history(
                    &c.0,
                    before.as_ref().map(|t| &*t.0),
                    None,
                    limit,
                    Priority::Visible,
                )
                .await
                .or_fail(&self.sh)?;
            Ok(self.page(&c, p))
        })
    }

    fn send(
        &self,
        c: &ConversationId,
        text: &str,
        thread: Option<MessageTs>,
        id: ClientMsgId,
    ) -> BoxFuture<'_, Result<Message, ProviderError>> {
        let (c, text) = (c.clone(), text.to_string());
        Box::pin(async move { self.post(&c, &text, thread.as_ref(), false, id).await })
    }

    fn edit(&self, id: &MessageId, text: &str) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (id, text) = (id.clone(), text.to_string());
        Box::pin(async move {
            self.sh
                .client
                .update_message(&id.conversation.0, &id.ts.0, &text)
                .await
                .or_fail(&self.sh)
        })
    }

    fn delete(&self, id: &MessageId) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = id.clone();
        Box::pin(async move {
            self.sh
                .client
                .delete_message(&id.conversation.0, &id.ts.0)
                .await
                .or_fail(&self.sh)
        })
    }

    fn mark_read(
        &self,
        c: &ConversationId,
        ts: &MessageTs,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (c, ts) = (c.clone(), ts.clone());
        Box::pin(async move {
            self.sh
                .client
                .mark_read(&c.0, &ts.0)
                .await
                .or_fail(&self.sh)
        })
    }

    fn events(&self) -> BoxStream<'static, ProviderEvent> {
        // The first stream starts at provider start so no early event is missed.
        let rx = self
            .sh
            .first_rx
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| self.sh.tx.subscribe());
        event_stream(rx)
    }

    fn history_between(
        &self,
        c: &ConversationId,
        after: MessageTs,
        before: Option<MessageTs>,
        limit: u32,
    ) -> BoxFuture<'_, Result<HistoryPage, ProviderError>> {
        let c = c.clone();
        Box::pin(async move {
            let p = self
                .sh
                .client
                .conversations_history(
                    &c.0,
                    before.as_ref().map(|t| &*t.0),
                    Some(&after.0),
                    limit,
                    Priority::Visible,
                )
                .await
                .or_fail(&self.sh)?;
            Ok(self.page(&c, p))
        })
    }

    fn replies(
        &self,
        c: &ConversationId,
        root: &MessageTs,
        after: Option<MessageTs>,
        limit: u32,
    ) -> BoxFuture<'_, Result<HistoryPage, ProviderError>> {
        let (c, root) = (c.clone(), root.clone());
        Box::pin(async move {
            let p = self
                .sh
                .client
                .conversations_replies(
                    &c.0,
                    &root.0,
                    after.as_ref().map(|t| &*t.0),
                    limit,
                    None,
                    Priority::Visible,
                )
                .await
                .or_fail(&self.sh)?;
            Ok(self.page(&c, p))
        })
    }

    fn reply(
        &self,
        c: &ConversationId,
        root: &MessageTs,
        text: &str,
        broadcast: bool,
        id: ClientMsgId,
    ) -> BoxFuture<'_, Result<Message, ProviderError>> {
        let (c, root, text) = (c.clone(), root.clone(), text.to_string());
        Box::pin(async move { self.post(&c, &text, Some(&root), broadcast, id).await })
    }

    fn react(
        &self,
        id: &MessageId,
        emoji: &EmojiRef,
        add: bool,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = id.clone();
        let name = match emoji.skin_tone {
            Some(t) => format!("{}::skin-tone-{t}", emoji.shortcode),
            None => emoji.shortcode.to_string(),
        };
        Box::pin(async move {
            let (c, ts) = (&id.conversation.0, &id.ts.0);
            let r = if add {
                self.sh.client.add_reaction(c, ts, &name).await
            } else {
                self.sh.client.remove_reaction(c, ts, &name).await
            };
            r.or_fail(&self.sh)
        })
    }

    fn schedule(
        &self,
        c: &ConversationId,
        thread: Option<MessageTs>,
        text: &str,
        post_at: i64,
    ) -> BoxFuture<'_, Result<ScheduledMessage, ProviderError>> {
        let (c, text) = (c.clone(), text.to_string());
        Box::pin(async move {
            let r = self
                .sh
                .client
                .schedule_message(&c.0, &text, post_at, thread.as_ref().map(|t| &*t.0))
                .await
                .or_fail(&self.sh)?;
            map_scheduled(r)
                .ok_or_else(|| ProviderError::Other("scheduled message without id".into()))
        })
    }

    fn scheduled(&self) -> BoxFuture<'_, Result<Vec<ScheduledMessage>, ProviderError>> {
        Box::pin(async move {
            let mut out = Vec::new();
            let mut cursor: Option<String> = None;
            // Bounded so a server that never ends the cursor chain cannot loop forever.
            for page_no in 0..MAX_SCHEDULED_PAGES {
                let (page, next) = self
                    .sh
                    .client
                    .scheduled_list(cursor.as_deref())
                    .await
                    .or_fail(&self.sh)?;
                out.extend(page.into_iter().filter_map(map_scheduled));
                match next {
                    Some(n) => {
                        if page_no + 1 == MAX_SCHEDULED_PAGES {
                            log::warn!(
                                "scheduled message list truncated after {MAX_SCHEDULED_PAGES} pages"
                            );
                        }
                        cursor = Some(n);
                    }
                    None => break,
                }
            }
            Ok(out)
        })
    }

    fn delete_scheduled(&self, s: &ScheduledMessage) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (c, id) = (s.conversation.clone(), s.id.clone());
        Box::pin(async move {
            self.sh
                .client
                .delete_scheduled(&c.0, &id.0)
                .await
                .or_fail(&self.sh)
        })
    }

    fn message(&self, id: &MessageId) -> BoxFuture<'_, Result<Message, ProviderError>> {
        let id = id.clone();
        Box::pin(async move {
            let m = self
                .sh
                .client
                .message_get(&id.conversation.0, &id.ts.0)
                .await
                .or_fail(&self.sh)?
                .ok_or_else(|| ProviderError::NotAllowed("message_not_found".into()))?;
            self.sh.note_bot(&m);
            let d = map_message_as(&id.conversation.0, m, Some(&self.sh.self_user));
            self.sh.index_files(&d);
            Ok(d)
        })
    }

    /// `conversations.info` for a channel the app does not hold (a live change
    /// came first): the answer maps like any conversation and is cached.
    fn conversation(
        &self,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<domain::Conversation, ProviderError>> {
        let id = c.0.to_string();
        Box::pin(async move {
            let raw = self
                .sh
                .client
                .conversations_info(&id)
                .await
                .or_fail(&self.sh)?;
            let conv = map_conversation(&raw, None, &|_| None);
            self.sh.cache_conversation(&conv);
            Ok(conv)
        })
    }

    fn pins(&self, c: &ConversationId) -> BoxFuture<'_, Result<Vec<Pin>, ProviderError>> {
        let c = c.clone();
        Box::pin(async move {
            let all = self.sh.client.pins_list(&c.0).await.or_fail(&self.sh)?;
            Ok(all.into_iter().map(|p| map_pin(&c.0, p)).collect())
        })
    }

    /// `already_pinned` and `no_pin` are the state the caller asked for.
    fn set_pinned(&self, id: &MessageId, pinned: bool) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = id.clone();
        Box::pin(async move {
            let (c, ts) = (&*id.conversation.0, &*id.ts.0);
            let r = if pinned {
                self.sh.client.pins_add(c, ts).await
            } else {
                self.sh.client.pins_remove(c, ts).await
            };
            match r {
                Err(ApiError::Slack { error, .. })
                    if error == if pinned { "already_pinned" } else { "no_pin" } =>
                {
                    Ok(())
                }
                r => r.or_fail(&self.sh),
            }
        })
    }

    fn bookmarks(&self, c: &ConversationId) -> BoxFuture<'_, Result<Vec<Bookmark>, ProviderError>> {
        let c = c.clone();
        Box::pin(async move {
            let all = self
                .sh
                .client
                .bookmarks_list(&c.0)
                .await
                .or_fail(&self.sh)?;
            Ok(all.into_iter().map(map_bookmark).collect())
        })
    }

    fn add_bookmark(
        &self,
        c: &ConversationId,
        title: &str,
        link: &str,
    ) -> BoxFuture<'_, Result<Bookmark, ProviderError>> {
        let (c, title, link) = (c.clone(), title.to_string(), link.to_string());
        Box::pin(async move {
            let b = self
                .sh
                .client
                .bookmarks_add(&c.0, &title, &link)
                .await
                .or_fail(&self.sh)?;
            Ok(map_bookmark(b))
        })
    }

    fn remove_bookmark(
        &self,
        c: &ConversationId,
        id: &BookmarkId,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (c, id) = (c.clone(), id.clone());
        Box::pin(async move {
            self.sh
                .client
                .bookmarks_remove(&c.0, &id.0)
                .await
                .or_fail(&self.sh)
        })
    }

    fn saved(
        &self,
        cursor: Option<Arc<str>>,
    ) -> BoxFuture<'_, Result<Page<SavedItem>, ProviderError>> {
        Box::pin(async move {
            let p = self
                .sh
                .client
                .saved_list(cursor.as_deref())
                .await
                .or_fail(&self.sh)?;
            Ok(Page {
                items: p.items.into_iter().map(map_saved).collect(),
                next: p.next.map(|c| Arc::from(c.as_str())),
            })
        })
    }

    fn set_saved(&self, id: &MessageId, saved: bool) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = id.clone();
        Box::pin(async move {
            let (c, ts) = (&*id.conversation.0, &*id.ts.0);
            let r = if saved {
                self.sh.client.saved_add(c, ts).await
            } else {
                self.sh.client.saved_remove(c, ts).await
            };
            r.or_fail(&self.sh)
        })
    }

    fn set_thread_followed(
        &self,
        root: &MessageId,
        follow: bool,
        last_read: Option<&MessageTs>,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let root = root.clone();
        let known = last_read.map(|t| t.0.to_string());
        Box::pin(async move {
            let (c, ts) = (&*root.conversation.0, &*root.ts.0);
            let client = &self.sh.client;
            // The read cursor: the one the store knows, else the subscription's (as the web
            // client reads it), else the thread ts. An unfollow does not fail on the read.
            let last_read = match known {
                Some(t) => t,
                None => match client.thread_subscription(c, ts).await {
                    Ok(t) => t.unwrap_or_else(|| ts.to_owned()),
                    Err(_) if !follow => ts.to_owned(),
                    Err(e) => return Err(self.sh.fail(&e)),
                },
            };
            let r = if follow {
                client.follow_thread(c, ts, &last_read).await
            } else {
                client.unfollow_thread(c, ts, &last_read).await
            };
            r.or_fail(&self.sh)
        })
    }

    fn thread_inbox(
        &self,
        cursor: Option<Arc<str>>,
    ) -> BoxFuture<'_, Result<Page<InboxThread>, ProviderError>> {
        Box::pin(async move {
            let p = self
                .sh
                .client
                .inbox_list(cursor.as_deref())
                .await
                .or_fail(&self.sh)?;
            let me = self.sh.self_user.as_str();
            Ok(Page {
                items: p.threads.into_iter().map(|t| map_inbox(t, me)).collect(),
                next: p.next.map(|c| Arc::from(c.as_str())),
            })
        })
    }

    fn reminders(&self) -> BoxFuture<'_, Result<Vec<Reminder>, ProviderError>> {
        Box::pin(async move {
            let all = self.sh.client.reminders_list().await.or_fail(&self.sh)?;
            Ok(all.into_iter().map(map_reminder).collect())
        })
    }

    fn add_reminder(
        &self,
        text: &str,
        when: &str,
        user: Option<&UserId>,
    ) -> BoxFuture<'_, Result<Reminder, ProviderError>> {
        let (text, when, user) = (text.to_string(), when.to_string(), user.cloned());
        Box::pin(async move {
            let r = self
                .sh
                .client
                .reminders_add(&text, &when, user.as_ref().map(|u| &*u.0))
                .await
                .or_fail(&self.sh)?;
            Ok(map_reminder(r))
        })
    }

    fn complete_reminder(&self, id: &ReminderId) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = id.clone();
        Box::pin(async move {
            self.sh
                .client
                .reminders_complete(&id.0)
                .await
                .map_err(|e| self.reminder_fail(&e))
        })
    }

    fn delete_reminder(&self, id: &ReminderId) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = id.clone();
        Box::pin(async move {
            self.sh
                .client
                .reminders_delete(&id.0)
                .await
                .map_err(|e| self.reminder_fail(&e))
        })
    }

    fn profile(&self, u: &UserId) -> BoxFuture<'_, Result<Profile, ProviderError>> {
        let u = u.clone();
        Box::pin(async move {
            if u.0.is_empty() {
                return Err(ProviderError::Other("empty user id".into()));
            }
            let raw = self.sh.client.profile_get(&u.0).await.or_fail(&self.sh)?;
            Ok(map_profile(raw))
        })
    }

    fn set_status(&self, s: Option<Status>) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(async move {
            let (text, emoji, expires) = match &s {
                None => (String::new(), String::new(), 0),
                Some(s) => (
                    s.text.to_string(),
                    s.emoji
                        .as_ref()
                        .map_or_else(String::new, |e| match e.skin_tone {
                            Some(t) => format!(":{}::skin-tone-{t}:", e.shortcode),
                            None => format!(":{}:", e.shortcode),
                        }),
                    s.expires.unwrap_or(0),
                ),
            };
            self.sh
                .client
                .profile_set_status(&text, &emoji, expires)
                .await
                .or_fail(&self.sh)
        })
    }

    fn set_presence(&self, p: PresenceSetting) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(async move {
            let mode = match p {
                PresenceSetting::Auto => PresenceMode::Auto,
                PresenceSetting::Away => PresenceMode::Away,
            };
            self.sh.client.set_presence(mode).await.or_fail(&self.sh)
        })
    }

    /// `presence_sub` for the people on screen (and a query for their current
    /// state); resent after a reconnect. The shape is in `realtime.md`.
    fn watch_presence(&self, users: &[UserId]) -> BoxFuture<'_, Result<(), ProviderError>> {
        let ids: Vec<String> = users.iter().map(|u| u.0.to_string()).collect();
        Box::pin(async move {
            *self.sh.presence_ids.lock().unwrap() = ids;
            self.sh.send_presence();
            Ok(())
        })
    }

    /// `conversations.open`: the direct message with `user`, created when new.
    fn open_direct(
        &self,
        user: &UserId,
    ) -> BoxFuture<'_, Result<domain::Conversation, ProviderError>> {
        let user = user.clone();
        Box::pin(async move {
            let raw = self
                .sh
                .client
                .conversations_open(&user.0)
                .await
                .or_fail(&self.sh)?;
            let mut c = map_conversation(&raw, None, &|_| None);
            c.kind = ConversationKind::Direct;
            c.peer = Some(user);
            c.is_open = true;
            self.sh.cache_conversation(&c);
            Ok(c)
        })
    }

    fn dnd(&self) -> BoxFuture<'_, Result<DndStatus, ProviderError>> {
        Box::pin(async move {
            let d = self.sh.client.dnd_info().await.or_fail(&self.sh)?;
            Ok(map_dnd(&d))
        })
    }

    fn dnd_users(
        &self,
        users: &[UserId],
    ) -> BoxFuture<'_, Result<HashMap<UserId, DndStatus>, ProviderError>> {
        let mut ids: Vec<String> = Vec::new();
        for u in users {
            let id = u.0.to_string();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Box::pin(async move {
            if ids.len() > DND_USERS_MAX {
                return Err(ProviderError::Other(format!(
                    "at most {DND_USERS_MAX} users per do-not-disturb lookup"
                )));
            }
            let got = self.sh.client.dnd_team_info(&ids).await.or_fail(&self.sh)?;
            Ok(got
                .into_iter()
                .map(|(u, d)| (UserId(Arc::from(u.as_str())), map_dnd(&d)))
                .collect())
        })
    }

    fn snooze(&self, minutes: u32) -> BoxFuture<'_, Result<DndStatus, ProviderError>> {
        Box::pin(async move {
            let snoozed = self
                .sh
                .client
                .dnd_set_snooze(minutes)
                .await
                .or_fail(&self.sh)?;
            // The snooze answer carries no schedule: read the whole state for the reducer.
            let mut full = self.sh.client.dnd_info().await.or_fail(&self.sh)?;
            full.snooze_enabled = snoozed.snooze_enabled;
            full.snooze_endtime = snoozed.snooze_endtime;
            Ok(map_dnd(&full))
        })
    }

    fn end_snooze(&self) -> BoxFuture<'_, Result<DndStatus, ProviderError>> {
        Box::pin(async move {
            match self.sh.client.dnd_end_snooze().await {
                Ok(_) => {}
                // Already ended (here or on another device): the goal holds.
                Err(ApiError::Slack { ref error, .. }) if error == "snooze_not_active" => {}
                Err(e) => return Err(self.sh.fail(&e)),
            }
            let d = self.sh.client.dnd_info().await.or_fail(&self.sh)?;
            Ok(map_dnd(&d))
        })
    }

    fn notify_prefs(
        &self,
    ) -> BoxFuture<'_, Result<HashMap<ConversationId, ConversationPrefs>, ProviderError>> {
        Box::pin(async move {
            let raw = self.sh.client.prefs_get().await.or_fail(&self.sh)?;
            Ok(map_notify_prefs(&raw))
        })
    }

    fn set_notify_prefs(
        &self,
        c: &ConversationId,
        p: ConversationPrefs,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let c = c.clone();
        Box::pin(async move {
            let _serial = self.sh.prefs_write.lock().await;
            self.sh
                .client
                .prefs_set_conversation(&c.0, level_value(p.level), p.muted)
                .await
                .or_fail(&self.sh)
        })
    }

    fn mark_thread_read(
        &self,
        c: &ConversationId,
        root: &MessageTs,
        ts: &MessageTs,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (c, root, ts) = (c.clone(), root.clone(), ts.clone());
        Box::pin(async move {
            self.sh
                .client
                .mark_thread(&c.0, &root.0, &ts.0)
                .await
                .or_fail(&self.sh)
        })
    }

    fn counts(&self) -> BoxFuture<'_, Result<Vec<ConversationCounts>, ProviderError>> {
        Box::pin(async move {
            let c = self.sh.client.counts().await.or_fail(&self.sh)?;
            Ok(c.channels
                .iter()
                .chain(&c.mpims)
                .chain(&c.ims)
                .map(map_counts)
                .collect())
        })
    }

    fn custom_emoji(&self) -> BoxFuture<'_, Result<Vec<CustomEmoji>, ProviderError>> {
        Box::pin(async move {
            let raw = self.sh.client.emoji_list().await.or_fail(&self.sh)?;
            Ok(resolve_emoji(&raw))
        })
    }

    fn user_groups(&self) -> BoxFuture<'_, Result<Vec<domain::UserGroup>, ProviderError>> {
        Box::pin(async move {
            let raw = self.sh.client.usergroups_list().await.or_fail(&self.sh)?;
            let me = self.sh.self_user.as_str();
            Ok(raw
                .into_iter()
                .map(|g| domain::UserGroup {
                    is_member: g.users.iter().any(|u| u == me),
                    id: g.id.into(),
                    handle: g.handle.into(),
                    name: g.name.into(),
                    member_count: g.user_count,
                })
                .collect())
        })
    }

    fn fetch(&self, url: &RemoteUrl, max: u64) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        let url = url.clone();
        Box::pin(async move {
            self.sh
                .client
                .fetch_private(&url.0, max)
                .await
                .or_fail(&self.sh)
        })
    }

    fn download(
        &self,
        url: &RemoteUrl,
        dest: &std::path::Path,
    ) -> BoxFuture<'_, Result<u64, ProviderError>> {
        let (url, dest) = (url.clone(), dest.to_path_buf());
        Box::pin(async move {
            self.sh
                .client
                .download_to(&url.0, &dest)
                .await
                .or_fail(&self.sh)
        })
    }

    fn upload(
        &self,
        c: &ConversationId,
        thread: Option<MessageTs>,
        files: Vec<Upload>,
        text: Option<&str>,
        id: ClientMsgId,
    ) -> BoxFuture<'_, Result<Message, ProviderError>> {
        let (c, text) = (c.clone(), text.map(str::to_string));
        Box::pin(async move { self.share_files(&c, thread, &files, text, id).await })
    }

    fn search_messages(
        &self,
        q: &domain::SearchQuery,
        cursor: Option<Arc<str>>,
        limit: u32,
    ) -> BoxFuture<'_, Result<domain::Page<domain::MessageHit>, ProviderError>> {
        let q = q.clone();
        Box::pin(async move { self.do_search_messages(&q, cursor, limit).await })
    }

    fn search_files(
        &self,
        q: &domain::SearchQuery,
        cursor: Option<Arc<str>>,
        limit: u32,
    ) -> BoxFuture<'_, Result<domain::Page<domain::FileHit>, ProviderError>> {
        let q = q.clone();
        Box::pin(async move { self.do_search_files(&q, cursor, limit).await })
    }

    fn search_people(
        &self,
        text: &str,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<domain::Member>, ProviderError>> {
        let text = text.to_string();
        Box::pin(async move { self.do_search_people(&text, limit).await })
    }

    fn members(&self, ids: &[UserId]) -> BoxFuture<'_, Result<Vec<domain::Member>, ProviderError>> {
        let ids = ids.to_vec();
        Box::pin(async move { self.do_members(&ids).await })
    }

    fn search_conversations(
        &self,
        text: &str,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<domain::Conversation>, ProviderError>> {
        let text = text.to_string();
        Box::pin(async move { self.do_search_conversations(&text, limit).await })
    }

    fn create_channel(
        &self,
        name: &str,
        private: bool,
    ) -> BoxFuture<'_, Result<domain::Conversation, ProviderError>> {
        let name = name.to_string();
        Box::pin(async move { self.do_create_channel(&name, private).await })
    }

    fn browse_channels(
        &self,
        query: &str,
        cursor: Option<Arc<str>>,
        limit: u32,
    ) -> BoxFuture<'_, Result<domain::Page<domain::Conversation>, ProviderError>> {
        let query = query.to_string();
        Box::pin(async move { self.do_browse(&query, cursor, limit).await })
    }

    fn join_conversation(
        &self,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<domain::Conversation, ProviderError>> {
        let c = c.clone();
        Box::pin(async move { self.do_join(&c).await })
    }

    fn leave_conversation(&self, c: &ConversationId) -> BoxFuture<'_, Result<(), ProviderError>> {
        let c = c.clone();
        Box::pin(async move { self.do_admin(AdminOp::Leave(&c.0)).await })
    }

    fn archive_conversation(&self, c: &ConversationId) -> BoxFuture<'_, Result<(), ProviderError>> {
        let c = c.clone();
        Box::pin(async move { self.do_admin(AdminOp::Archive(&c.0)).await })
    }

    fn rename_conversation(
        &self,
        c: &ConversationId,
        name: &str,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (c, name) = (c.clone(), name.to_string());
        Box::pin(async move { self.do_admin(AdminOp::Rename(&c.0, &name)).await })
    }

    fn set_topic(
        &self,
        c: &ConversationId,
        topic: &str,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (c, topic) = (c.clone(), topic.to_string());
        Box::pin(async move { self.do_admin(AdminOp::Topic(&c.0, &topic)).await })
    }

    fn set_purpose(
        &self,
        c: &ConversationId,
        purpose: &str,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (c, purpose) = (c.clone(), purpose.to_string());
        Box::pin(async move { self.do_admin(AdminOp::Purpose(&c.0, &purpose)).await })
    }

    fn invite(
        &self,
        c: &ConversationId,
        users: &[UserId],
    ) -> BoxFuture<'_, Result<domain::InviteReport, ProviderError>> {
        let (c, users) = (c.clone(), users.to_vec());
        Box::pin(async move { self.do_invite(&c.0, &users).await })
    }

    fn conversation_members(
        &self,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<Vec<UserId>, ProviderError>> {
        let c = c.clone();
        Box::pin(async move { self.do_channel_members(&c.0).await })
    }

    fn check_channel_name(&self, typed: &str) -> domain::ChannelName {
        crate::channel_admin::check_channel_name(typed)
    }

    fn set_conversation_starred(
        &self,
        c: &ConversationId,
        starred: bool,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let c = c.clone();
        Box::pin(async move { self.do_admin(AdminOp::Star(&c.0, starred)).await })
    }

    fn huddles(&self) -> BoxFuture<'_, Result<Vec<Huddle>, ProviderError>> {
        Box::pin(async move {
            let rooms = self
                .sh
                .client
                .huddles_list(&self.ws.id.0, Priority::Visible)
                .await
                .or_fail(&self.sh)?;
            Ok(rooms.iter().filter_map(map_room).collect())
        })
    }

    fn decline_huddle(
        &self,
        c: &ConversationId,
        id: &HuddleId,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (c, id) = (c.clone(), id.clone());
        Box::pin(async move {
            self.sh
                .client
                .huddles_decline(&c.0, &id.0)
                .await
                .or_fail(&self.sh)
        })
    }

    fn huddle_url(&self, c: &ConversationId) -> Option<String> {
        slack_auth::webview::huddle_url(&self.ws.id.0, &c.0).map(String::from)
    }

    fn app_url(&self, c: &ConversationId) -> Option<String> {
        // The huddle URL's id checks apply, so a malformed id builds nothing.
        let u = slack_auth::webview::huddle_url(&self.ws.id.0, &c.0)?;
        Some(u.as_str().replacen("/huddle/", "/client/", 1))
    }

    fn call_surface(
        &self,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<CallSurface, ProviderError>> {
        let ws = self.ws.clone();
        let client = self.sh.client.clone();
        let c = c.clone();
        let title = self.call_title(&c);
        Box::pin(async move {
            let url = slack_auth::webview::huddle_url(&ws.id.0, &c.0)
                .ok_or_else(|| ProviderError::Other("malformed conversation id".into()))?;
            let cookie = client.session_cookie();
            let mut cookies = vec![surface_cookie("d", cookie.d_value())];
            if let Some(ds) = cookie.d_s_value() {
                cookies.push(surface_cookie("d-s", ds));
            }
            let suffixes: Arc<[Arc<str>]> =
                Arc::from([Arc::from(slack_auth::webview::CALL_HOST_SUFFIX)]);
            let signed_out_paths: Arc<[Arc<str>]> = slack_auth::webview::CALL_SIGNED_OUT_PATHS
                .iter()
                .copied()
                .map(Arc::from)
                .collect();
            #[cfg(target_os = "macos")]
            let user_agent = Some(Arc::from(slack_auth::webview::safari_user_agent(
                slack_auth::webview::os_major(),
            )));
            #[cfg(not(target_os = "macos"))]
            let user_agent = Some(Arc::from(slack_auth::webview::safari_user_agent(26)));
            Ok(CallSurface {
                url: Arc::from(url.as_str()),
                title,
                cookies,
                allowed_host_suffixes: suffixes,
                signed_out_paths,
                user_agent,
            })
        })
    }

    fn native_call_join(
        &self,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<NativeCallJoin, ProviderError>> {
        let c = c.clone();
        Box::pin(async move {
            // The huddle URL's id checks apply, so a malformed id sends nothing.
            slack_auth::webview::huddle_url(&self.ws.id.0, &c.0)
                .ok_or_else(|| ProviderError::Other("malformed conversation id".into()))?;
            let j = self.sh.client.rooms_join(&c.0).await.or_fail(&self.sh)?;
            Ok(NativeCallJoin {
                meeting_id: Arc::from(j.meeting_id),
                attendee_id: Arc::from(j.attendee_id),
                external_user_id: Arc::from(j.external_user_id),
                join_token: SecretText::new(j.join_token.expose()),
                signaling_url: Arc::from(j.signaling_url),
                audio_host_url: Arc::from(j.audio_host_url),
                turn_control_url: j.turn_control_url.map(Arc::from),
            })
        })
    }

    fn web_url(&self, id: &MessageId) -> Option<String> {
        ts_opt(&id.ts.0)?;
        Some(format!(
            "{}/archives/{}/p{}",
            self.sh.origin,
            id.conversation.0,
            id.ts.0.replace('.', "")
        ))
    }

    /// The channel's web client page (`/archives/<id>`), the link the web
    /// client's own Copy link gives a conversation.
    fn conversation_web_url(&self, c: &ConversationId) -> Option<String> {
        (!c.0.is_empty()).then(|| format!("{}/archives/{}", self.sh.origin, c.0))
    }

    fn web_origin(&self) -> Option<String> {
        Some(self.sh.origin.clone())
    }

    fn reconnect_now(&self) {
        self.sh.retry_realtime();
    }

    /// `user_typing` on the session socket; dropped while it is down. The
    /// `thread_ts` field is unverified (realtime.md).
    fn typing(&self, conversation: &ConversationId, thread: Option<&MessageTs>) {
        if let Some(h) = self.sh.realtime.lock().unwrap().as_ref() {
            h.send(Outgoing::Typing {
                id: h.next_id(),
                channel: conversation.0.to_string(),
                thread_ts: thread.map(|t| t.0.to_string()),
            });
        }
    }
}

pub(crate) fn ext_codec() -> Arc<dyn ExtCodec> {
    ext::codec()
}

pub(crate) fn content_parser() -> Arc<dyn ContentParser> {
    ext::parser()
}

/// `None` for an entry without an id (it could not be deleted or matched later).
fn map_scheduled(r: slack_api::ScheduledRaw) -> Option<ScheduledMessage> {
    if r.id.is_empty() {
        return None;
    }
    Some(ScheduledMessage {
        id: ScheduleId(r.id.into()),
        conversation: ConversationId(r.channel_id.as_str().into()),
        thread: r.thread_ts.map(|t| MessageTs(t.as_str().into())),
        text: r.text.into(),
        post_at: r.post_at,
    })
}

fn map_reminder(r: slack_api::ReminderRaw) -> Reminder {
    Reminder {
        id: ReminderId(r.id.into()),
        text: r.text.into(),
        due: r.time,
        recurring: r.recurring,
        completed: r.complete_ts.is_some(),
        fired: false,
        interval_secs: 0,
        for_user: UserId(r.user.into()),
        link: None,
        snoozed_until: None,
    }
}

/// Events from the broadcast; a lagged receiver lost events, so it yields a
/// reconnect-style `Connected` and the domain resyncs.
fn event_stream(rx: broadcast::Receiver<ProviderEvent>) -> BoxStream<'static, ProviderEvent> {
    futures::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Ok(e) => Some((e, rx)),
            Err(broadcast::error::RecvError::Lagged(_)) => {
                Some((ProviderEvent::Connected { reconnect: true }, rx))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    })
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lagged_receiver_yields_reconnect() {
        let (tx, rx) = broadcast::channel(2);
        for _ in 0..5 {
            tx.send(ProviderEvent::Disconnected).unwrap();
        }
        let mut s = event_stream(rx);
        assert!(matches!(
            s.next().await,
            Some(ProviderEvent::Connected { reconnect: true })
        ));
        assert!(matches!(s.next().await, Some(ProviderEvent::Disconnected)));
    }
}
