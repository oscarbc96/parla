//! `FakeHost` (feature `testing`, native targets only): runs a plugin's Rust
//! code against in-memory host functions, so its logic is unit tested with
//! plain `cargo test` and no wasm runtime.
//!
//! ```ignore
//! let fake = FakeHost::from_manifest(include_str!("../plugin.toml"));
//! MyPlugin::run_command("greet".into(), context).unwrap();
//! assert_eq!(fake.toasts()[0].0, "Hello");
//! ```
//!
//! The fake holds the plugin's grants ([`Grants`], from its `plugin.toml` or
//! built by hand; [`FakeHost::new`] grants nothing) and every guarded call
//! without its grant returns `denied`. It checks the documented sizes,
//! counts and per-minute rates of `storage`, `secrets`, `ui`, `host.log`,
//! `host.t`, `exec`, `http` and `workspace`, rejects trees the way
//! [`view::validate`](crate::view::validate) does, refuses `update`,
//! `patch`, `close` and `refresh-after` on views that are not open, and
//! keeps what the plugin did for the test to read. Rates use the fake clock
//! ([`FakeHost::set_now`], [`FakeHost::advance`]); a refused call does not
//! count toward a rate. [`FakeHost::inject`] makes any call fail.
//!
//! Workspace reads go to per-method stubs (`on_history`, ...); an unset
//! read returns an empty page or list, `not-found` for a single item, an
//! empty status and `unknown` presence. Writes are recorded
//! ([`FakeHost::writes`]). `http` and `exec` go to [`FakeHost::on_http`]
//! and [`FakeHost::on_exec`]; without a handler `http` is `offline` and
//! `exec` is `failed`.
//!
//! Not enforced by the fake (the host does these):
//! - `dms:read` for direct and group conversations (the fake does not know
//!   a conversation's kind).
//! - Concurrency caps: 4 `http` requests and 2 `exec` runs at once (fake
//!   calls are synchronous).
//! - `ui.update` coalescing to 10 per second, quiet hours for `notify`, and
//!   the target of `notify`.
//! - Loopback and private addresses in `http` beyond the granted host list.
//! - Redirects, cookies and the `User-Agent` the host sets on `http`.
//! - The `exec` environment, working directory, wall clock and output caps.
//! - Fluent syntax in `host.t` beyond `{ $name }` placeholders.
//! - The 1 MB `mcp-call` and 4 KB slash `args` caps, which bound what the
//!   host sends, not what the plugin calls.
//! - The 16 MB default memory cap and call deadlines.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

use crate::host::LogLevel;
use crate::host::exec::ExecOutput;
use crate::host::http::{Request, Response};
use crate::host::settings::SettingValue;
use crate::host::storage::Scope;
use crate::host::ui::{NotifyTarget, ToastKind, WindowSpec};
use crate::types::*;
use crate::view::{self, NodeKind, ViewTree};

const MIB: usize = 1024 * 1024;
const MINUTE_MS: i64 = 60_000;

/// What the plugin may do: the manifest's `[capabilities]`
/// and its memory cap. Build one with [`Grants::none`] and the setters, or
/// read it from a manifest with [`Grants::from_manifest`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct Grants {
    pub workspace_read: BTreeSet<String>,
    pub workspace_write: BTreeSet<String>,
    /// Exact hosts, lower case.
    pub network: BTreeSet<String>,
    pub storage: bool,
    pub windows: bool,
    pub panel: bool,
    pub notifications: bool,
    pub clipboard_write: bool,
    pub exec: Vec<ExecGrant>,
    /// `[limits] memory_mb`; ranged reads and responses are capped at a
    /// quarter of it.
    pub memory_mb: u32,
}

/// One `[[capabilities.exec]]` entry: a program and its allowed argv shapes
/// (`*` matches within one argument).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct ExecGrant {
    pub program: String,
    pub allow: Vec<Vec<String>>,
}

impl Default for Grants {
    fn default() -> Self {
        Self::none()
    }
}

impl Grants {
    /// No capabilities: only calls that need none work.
    pub fn none() -> Self {
        Self {
            workspace_read: BTreeSet::new(),
            workspace_write: BTreeSet::new(),
            network: BTreeSet::new(),
            storage: false,
            windows: false,
            panel: false,
            notifications: false,
            clipboard_write: false,
            exec: Vec::new(),
            memory_mb: 16,
        }
    }
    /// A `workspace_read` area such as `messages:read`.
    pub fn read(mut self, area: &str) -> Self {
        self.workspace_read.insert(area.into());
        self
    }
    /// A `workspace_write` area such as `messages:write`.
    pub fn write(mut self, area: &str) -> Self {
        self.workspace_write.insert(area.into());
        self
    }
    pub fn network(mut self, host: &str) -> Self {
        self.network.insert(host.to_ascii_lowercase());
        self
    }
    pub fn storage(mut self) -> Self {
        self.storage = true;
        self
    }
    pub fn windows(mut self) -> Self {
        self.windows = true;
        self
    }
    pub fn panel(mut self) -> Self {
        self.panel = true;
        self
    }
    pub fn notifications(mut self) -> Self {
        self.notifications = true;
        self
    }
    pub fn clipboard_write(mut self) -> Self {
        self.clipboard_write = true;
        self
    }
    /// `program` with its argv shapes, for example `&[&["pr", "view", "*"]]`.
    pub fn exec(mut self, program: &str, allow: &[&[&str]]) -> Self {
        self.exec.push(ExecGrant {
            program: program.into(),
            allow: allow
                .iter()
                .map(|s| s.iter().map(|a| a.to_string()).collect())
                .collect(),
        });
        self
    }
    pub fn memory_mb(mut self, mb: u32) -> Self {
        self.memory_mb = mb;
        self
    }

    /// The grants a `plugin.toml` declares, as if the user accepted them all.
    pub fn from_manifest(src: &str) -> Result<Self, String> {
        Ok(parse_manifest(src)?.grants)
    }
}

/// What a manifest declares beyond grants.
struct Declared {
    grants: Grants,
    settings: BTreeMap<String, Option<SettingValue>>,
    panels: BTreeSet<String>,
}

fn parse_manifest(src: &str) -> Result<Declared, String> {
    let doc: toml::Table = src.parse().map_err(|e| format!("plugin.toml: {e}"))?;
    let mut g = Grants::none();
    let strs = |v: Option<&toml::Value>| -> Vec<String> {
        v.and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect()
    };
    if let Some(caps) = doc.get("capabilities").and_then(toml::Value::as_table) {
        g.workspace_read = strs(caps.get("workspace_read")).into_iter().collect();
        g.workspace_write = strs(caps.get("workspace_write")).into_iter().collect();
        g.network = strs(caps.get("network"))
            .into_iter()
            .map(|h| h.to_ascii_lowercase())
            .collect();
        let flag = |k: &str| caps.get(k).and_then(toml::Value::as_bool).unwrap_or(false);
        g.storage = flag("storage");
        g.windows = flag("windows");
        g.panel = flag("panel");
        g.notifications = flag("notifications");
        g.clipboard_write = flag("clipboard_write");
        for rule in caps
            .get("exec")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            let program = rule
                .get("program")
                .and_then(toml::Value::as_str)
                .ok_or("exec rule without program")?;
            let allow = rule
                .get("allow")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .map(|shape| strs(Some(shape)))
                .collect();
            g.exec.push(ExecGrant {
                program: program.into(),
                allow,
            });
        }
    }
    if let Some(mb) = doc
        .get("limits")
        .and_then(|l| l.get("memory_mb"))
        .and_then(toml::Value::as_integer)
    {
        g.memory_mb = mb as u32;
    }
    let mut settings = BTreeMap::new();
    for s in doc
        .get("settings")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = s
            .get("key")
            .and_then(toml::Value::as_str)
            .ok_or("setting without key")?;
        let kind = s
            .get("type")
            .and_then(toml::Value::as_str)
            .unwrap_or("string");
        let default = s.get("default").and_then(|d| match (kind, d) {
            ("boolean", toml::Value::Boolean(b)) => Some(SettingValue::Toggle(*b)),
            ("integer", toml::Value::Integer(i)) => Some(SettingValue::Number(*i as f64)),
            ("enum", toml::Value::String(c)) => Some(SettingValue::Choice(c.clone())),
            ("string", toml::Value::String(t)) => Some(SettingValue::Text(t.clone())),
            _ => None,
        });
        // A `secret` setting lives in `secrets`, not `settings`.
        if kind != "secret" {
            settings.insert(key.to_string(), default);
        }
    }
    let panels = doc
        .get("panels")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| {
            p.get("id")
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        })
        .collect();
    Ok(Declared {
        grants: g,
        settings,
        panels,
    })
}

/// What the plugin is handling, which decides the calls that need a user
/// action and the writes event hooks may not make.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Invocation {
    /// A command, slash command, message action or a click in a view: the
    /// default.
    #[default]
    UserAction,
    /// Activation, a renderer, a refresh or anything else without a user
    /// action: no `copy`, `open-url` or `open-conversation`, and `toast` only
    /// with a view open.
    Background,
    /// An event hook: as `Background`, and workspace writes are `denied`.
    EventHook,
}

/// One `ui` call the plugin made, in order. Refused calls are not kept.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum UiCall {
    OpenPanel {
        panel: String,
        view: ViewId,
        tree: ViewTree,
    },
    OpenWindow {
        spec: WindowSpec,
        view: ViewId,
        tree: ViewTree,
    },
    Update {
        view: ViewId,
        tree: ViewTree,
    },
    Patch {
        view: ViewId,
        node: NodeId,
        tree: ViewTree,
    },
    Close {
        view: ViewId,
    },
    RefreshAfter {
        view: ViewId,
        seconds: u32,
    },
    Toast {
        text: String,
        kind: ToastKind,
    },
    Notify {
        title: String,
        body: String,
        target: NotifyTarget,
    },
    Copy {
        text: String,
    },
    OpenUrl {
        url: String,
    },
    OpenConversation {
        workspace: WorkspaceId,
        conversation: ConversationId,
        message: Option<MessageId>,
    },
}

/// A message the plugin sent with `workspace.send` or `workspace.reply`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct Sent {
    pub workspace: WorkspaceId,
    pub conversation: ConversationId,
    /// The thread of a reply.
    pub thread: Option<MessageId>,
    pub content: Content,
    /// A reply also posted to the channel.
    pub also_to_channel: bool,
}

/// One workspace write the plugin made, in order. Refused writes are not
/// kept.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum WriteCall {
    Send(Sent),
    Reply(Sent),
    Edit {
        workspace: WorkspaceId,
        message: MessageRef,
        content: Content,
    },
    Delete {
        workspace: WorkspaceId,
        message: MessageRef,
    },
    React {
        workspace: WorkspaceId,
        message: MessageRef,
        emoji: String,
    },
    Unreact {
        workspace: WorkspaceId,
        message: MessageRef,
        emoji: String,
    },
    AddReminder {
        workspace: WorkspaceId,
        text: String,
        at: Timestamp,
    },
    CompleteReminder {
        workspace: WorkspaceId,
        reminder: ReminderId,
    },
    DeleteReminder {
        workspace: WorkspaceId,
        reminder: ReminderId,
    },
    Schedule {
        workspace: WorkspaceId,
        conversation: ConversationId,
        content: Content,
        at: Timestamp,
    },
    CancelScheduled {
        workspace: WorkspaceId,
        scheduled: ScheduledId,
    },
    Upload {
        workspace: WorkspaceId,
        conversation: ConversationId,
        name: String,
        bytes: Vec<u8>,
        comment: Option<String>,
    },
    SetStatus {
        workspace: WorkspaceId,
        status: Status,
    },
    SetPresence {
        workspace: WorkspaceId,
        presence: Presence,
    },
    Snooze {
        workspace: WorkspaceId,
        minutes: u32,
    },
    EndSnooze {
        workspace: WorkspaceId,
    },
    SetSaved {
        workspace: WorkspaceId,
        message: MessageRef,
        saved: bool,
    },
    SetPinned {
        workspace: WorkspaceId,
        message: MessageRef,
        pinned: bool,
    },
    MarkRead {
        workspace: WorkspaceId,
        conversation: ConversationId,
        upto: MessageId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewKind {
    Panel,
    Window,
    /// Assigned by the host: a rendered link or a slash note.
    Host,
}

struct OpenView {
    kind: ViewKind,
    tree: Option<ViewTree>,
}

type HttpHandler = Box<dyn FnMut(&Request) -> Result<Response, Error>>;
type ExecHandler = Box<dyn FnMut(&str, &[String]) -> Result<ExecOutput, Error>>;

/// Declares the stubbable workspace reads: the stub field, its `on_` setter
/// and the fake call (injection, grant and argument checks, the read rate,
/// then the stub or the empty default).
macro_rules! reads {
    ($(
        $(#[$m:meta])*
        fn $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty, $setter:ident,
            check: $check:block, empty: $empty:expr;
    )*) => {
        #[derive(Default)]
        struct ReadStubs {
            $( $name: Option<Box<dyn FnMut($($ty),*) -> Result<$ret, Error>>>, )*
        }

        impl FakeHost {
            $(
                $(#[$m])*
                #[doc = concat!("Stubs `workspace.", stringify!($name), "` (after the grant and argument checks).")]
                pub fn $setter(&self, stub: impl FnMut($($ty),*) -> Result<$ret, Error> + 'static) {
                    self.set(|s| s.reads.$name = Some(Box::new(stub)));
                }
            )*
        }

        mod reads_imp {
            use super::*;
            $(
                pub fn $name($($arg: $ty),*) -> Result<$ret, Error> {
                    gate(concat!("workspace.", stringify!($name)))?;
                    $check
                    rate("workspace.read", 60)?;
                    let st = state();
                    let stub = st.borrow_mut().reads.$name.take();
                    let Some(mut stub) = stub else { return $empty };
                    let out = stub($($arg),*);
                    st.borrow_mut().reads.$name = Some(stub);
                    out
                }
            )*
        }
    };
}

fn empty_page<T>() -> (Vec<T>, Cursor) {
    (Vec::new(), None)
}

reads! {
    fn list_workspaces() -> Vec<Workspace>, on_list_workspaces,
        check: { area("workspace:read")?; }, empty: Ok(Vec::new());
    fn me(ws: &str) -> User, on_me,
        check: { let _ = ws; area("workspace:read")?; }, empty: Err(Error::NotFound);
    fn list_conversations(ws: &str, filter: &ConversationFilter, cursor: Option<&str>, limit: u32) -> ConversationPage, on_list_conversations,
        check: { let _ = (ws, filter, cursor); area("conversations:read")?; page_limit(limit)?; if let Some(q) = &filter.query { query(q)?; } },
        empty: { let (items, next) = empty_page(); Ok(ConversationPage { items, next }) };
    fn get_conversation(ws: &str, conversation: &str) -> Conversation, on_get_conversation,
        check: { let _ = (ws, conversation); area("conversations:read")?; }, empty: Err(Error::NotFound);
    fn history(ws: &str, conversation: &str, cursor: Option<&str>, limit: u32) -> MessagePage, on_history,
        check: { let _ = (ws, cursor); history_area(conversation)?; page_limit(limit)?; },
        empty: { let (items, next) = empty_page(); Ok(MessagePage { items, next }) };
    fn replies(ws: &str, conversation: &str, thread: &str, cursor: Option<&str>, limit: u32) -> MessagePage, on_replies,
        check: { let _ = (ws, thread, cursor); history_area(conversation)?; page_limit(limit)?; },
        empty: { let (items, next) = empty_page(); Ok(MessagePage { items, next }) };
    fn get_message(ws: &str, message: &MessageRef) -> Message, on_get_message,
        check: { let _ = ws; history_area(&message.conversation)?; }, empty: Err(Error::NotFound);
    fn permalink(ws: &str, message: &MessageRef) -> String, on_permalink,
        check: { let _ = ws; history_area(&message.conversation)?; }, empty: Err(Error::NotFound);
    fn get_user(ws: &str, user: &str) -> User, on_get_user,
        check: { let _ = (ws, user); area("people:read")?; }, empty: Err(Error::NotFound);
    fn find_users(ws: &str, q: &str, limit: u32) -> Vec<User>, on_find_users,
        check: { let _ = ws; area("people:read")?; query(q)?; page_limit(limit)?; }, empty: Ok(Vec::new());
    fn get_presence(ws: &str, user: &str) -> Presence, on_get_presence,
        check: { let _ = (ws, user); area("people:read")?; }, empty: Ok(Presence::Unknown);
    fn search_messages(ws: &str, q: &str, cursor: Option<&str>, limit: u32) -> MessagePage, on_search_messages,
        check: { let _ = (ws, cursor); area("search:read")?; query(q)?; page_limit(limit)?; },
        empty: { let (items, next) = empty_page(); Ok(MessagePage { items, next }) };
    fn search_files(ws: &str, q: &str, cursor: Option<&str>, limit: u32) -> FilePage, on_search_files,
        check: { let _ = (ws, cursor); area("search:read")?; query(q)?; page_limit(limit)?; },
        empty: { let (items, next) = empty_page(); Ok(FilePage { items, next }) };
    fn file_info(ws: &str, file: &str) -> File, on_file_info,
        check: { let _ = (ws, file); area("files:read")?; }, empty: Err(Error::NotFound);
    fn file_bytes(ws: &str, file: &str, offset: u64, len: u32) -> Vec<u8>, on_file_bytes,
        check: {
            let _ = (ws, file, offset);
            area("files:read")?;
            let cap = quarter_memory();
            if len as usize > cap { return Err(Error::Limit(format!("file-bytes len {len} over {cap}"))); }
        },
        empty: Err(Error::NotFound);
    fn list_reminders(ws: &str, cursor: Option<&str>, limit: u32) -> ReminderPage, on_list_reminders,
        check: { let _ = (ws, cursor); area("reminders:read")?; page_limit(limit)?; },
        empty: { let (items, next) = empty_page(); Ok(ReminderPage { items, next }) };
    fn list_scheduled(ws: &str, cursor: Option<&str>, limit: u32) -> ScheduledPage, on_list_scheduled,
        check: { let _ = (ws, cursor); area("scheduled:read")?; page_limit(limit)?; },
        empty: { let (items, next) = empty_page(); Ok(ScheduledPage { items, next }) };
    fn list_saved(ws: &str, cursor: Option<&str>, limit: u32) -> MessageRefPage, on_list_saved,
        check: { let _ = (ws, cursor); area("saved:read")?; page_limit(limit)?; },
        empty: { let (items, next) = empty_page(); Ok(MessageRefPage { items, next }) };
    fn list_pins(ws: &str, conversation: &str, cursor: Option<&str>, limit: u32) -> MessageRefPage, on_list_pins,
        check: { let _ = (ws, conversation, cursor); area("pins:read")?; page_limit(limit)?; },
        empty: { let (items, next) = empty_page(); Ok(MessageRefPage { items, next }) };
    fn my_status(ws: &str) -> Status, on_my_status,
        check: { let _ = ws; area("status:read")?; },
        empty: Ok(Status { text: String::new(), emoji: None, expires: None, snoozed_until: None });
}

struct State {
    grants: Grants,
    settings: BTreeMap<String, Option<SettingValue>>,
    panels: BTreeSet<String>,
    invocation: Invocation,
    dev_mode: bool,
    injections: BTreeMap<String, (Error, bool)>,
    rates: BTreeMap<&'static str, VecDeque<Timestamp>>,
    app_info: AppInfo,
    now: Timestamp,
    strings: BTreeMap<String, String>,
    logs: Vec<(LogLevel, String)>,
    context: Context,
    reads: ReadStubs,
    writes: Vec<WriteCall>,
    storage: BTreeMap<(Option<WorkspaceId>, String), Vec<u8>>,
    secrets: BTreeMap<String, String>,
    ui: Vec<UiCall>,
    views: BTreeMap<ViewId, OpenView>,
    refreshes: BTreeSet<ViewId>,
    next_id: u32,
    draft: Option<String>,
    http: Option<HttpHandler>,
    http_requests: Vec<Request>,
    exec: Option<ExecHandler>,
}

impl State {
    fn new(d: Declared) -> Self {
        Self {
            grants: d.grants,
            settings: d.settings,
            panels: d.panels,
            invocation: Invocation::UserAction,
            dev_mode: false,
            injections: BTreeMap::new(),
            rates: BTreeMap::new(),
            app_info: AppInfo {
                app_version: "0.0.0".into(),
                platform: "test".into(),
                locale: "en".into(),
                appearance: Appearance::Light,
                density: Density::Cozy,
            },
            now: 0,
            strings: BTreeMap::new(),
            logs: Vec::new(),
            context: Context {
                workspace: None,
                conversation: None,
                thread: None,
                message: None,
            },
            reads: ReadStubs::default(),
            writes: Vec::new(),
            storage: BTreeMap::new(),
            secrets: BTreeMap::new(),
            ui: Vec::new(),
            views: BTreeMap::new(),
            refreshes: BTreeSet::new(),
            next_id: 0,
            draft: None,
            http: None,
            http_requests: Vec::new(),
            exec: None,
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
}

fn state() -> Rc<RefCell<State>> {
    CURRENT.with(|c| c.borrow().clone()).expect(
        "no FakeHost on this thread: create one with FakeHost::new() or FakeHost::from_manifest()",
    )
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(&mut state().borrow_mut())
}

/// Fails the call if a test injected an error for it.
fn gate(call: &str) -> Result<(), Error> {
    with(|s| match s.injections.get(call) {
        Some((e, once)) => {
            let e = e.clone();
            if *once {
                s.injections.remove(call);
            }
            Err(e)
        }
        None => Ok(()),
    })
}

/// Counts one call against a per-minute rate on the fake clock.
fn rate(bucket: &'static str, per_minute: usize) -> Result<(), Error> {
    with(|s| {
        let now = s.now;
        let q = s.rates.entry(bucket).or_default();
        while q.front().is_some_and(|t| *t <= now - MINUTE_MS) {
            q.pop_front();
        }
        if q.len() >= per_minute {
            return Err(Error::Limit(format!("{bucket}: {per_minute} per minute")));
        }
        q.push_back(now);
        Ok(())
    })
}

fn denied(what: &str) -> Error {
    Error::Denied(what.into())
}

fn too_big(what: &str, len: usize, max: usize) -> Result<(), Error> {
    if len > max {
        Err(Error::Limit(format!("{what}: {len} bytes, at most {max}")))
    } else {
        Ok(())
    }
}

fn area(name: &str) -> Result<(), Error> {
    with(|s| {
        if s.grants.workspace_read.contains(name) {
            Ok(())
        } else {
            Err(denied(name))
        }
    })
}

/// `messages:read` covers the conversation in context; `history:read` any.
fn history_area(conversation: &str) -> Result<(), Error> {
    with(|s| {
        let r = &s.grants.workspace_read;
        let in_context = s.context.conversation.as_deref() == Some(conversation);
        if r.contains("history:read") || (in_context && r.contains("messages:read")) {
            Ok(())
        } else if in_context {
            Err(denied("messages:read"))
        } else {
            Err(denied("history:read"))
        }
    })
}

fn page_limit(limit: u32) -> Result<(), Error> {
    if (1..=200).contains(&limit) {
        Ok(())
    } else {
        Err(Error::Invalid(format!("limit {limit} not in 1 to 200")))
    }
}

fn query(q: &str) -> Result<(), Error> {
    too_big("query", q.len(), 256)
}

fn quarter_memory() -> usize {
    with(|s| s.grants.memory_mb as usize * MIB / 4)
}

fn user_action(what: &str) -> Result<(), Error> {
    with(|s| {
        if s.invocation == Invocation::UserAction {
            Ok(())
        } else {
            Err(Error::Denied(format!(
                "{what} only while handling a user action"
            )))
        }
    })
}

/// The host of an `https://` URL, lower case, without port or user info.
fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = if host.starts_with('[') {
        host.split(']').next()?.trim_start_matches('[')
    } else {
        host.split(':').next()?
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn granted_host(url: &str) -> Result<(), Error> {
    let host =
        https_host(url).ok_or_else(|| Error::Invalid(format!("`{url}` is not an https URL")))?;
    with(|s| {
        if s.grants.network.contains(&host) {
            Ok(())
        } else {
            Err(Error::Denied(format!("network: {host}")))
        }
    })
}

/// `view::validate` plus the image host rule the host adds (rule 10).
fn check_tree(tree: &ViewTree) -> Result<(), Error> {
    view::validate(tree)?;
    for n in &tree.nodes {
        if let NodeKind::Image(img) = &n.kind
            && img.src.starts_with("https://")
        {
            granted_host(&img.src)?;
        }
    }
    Ok(())
}

fn scope_key(scope: &Scope) -> Option<WorkspaceId> {
    match scope {
        Scope::Global => None,
        Scope::Workspace(ws) => Some(ws.clone()),
    }
}

fn glob(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((head, tail)) => {
            let Some(rest) = text.strip_prefix(head) else {
                return false;
            };
            (0..=rest.len())
                .filter(|i| rest.is_char_boundary(*i))
                .any(|i| glob(tail, &rest[i..]))
        }
    }
}

fn content_ok(content: &Content) -> Result<(), Error> {
    let text: usize = content
        .spans
        .iter()
        .map(|i| match i {
            Inline::Text(t) => t.len(),
            Inline::Link(l) => l.url.len() + l.label.as_ref().map_or(0, String::len),
            _ => 0,
        })
        .sum();
    too_big("content text", text, 16 * 1024)?;
    if let Some(url) = &content.image_url
        && !url.starts_with("https://")
    {
        return Err(Error::Invalid("image-url takes https only".into()));
    }
    Ok(())
}

/// The in-memory host for this thread. Created with [`FakeHost::new`],
/// [`FakeHost::with_grants`] or [`FakeHost::from_manifest`], it takes every
/// host call made on this thread until it is dropped.
pub struct FakeHost {
    state: Rc<RefCell<State>>,
}

impl FakeHost {
    /// A fake host with no grants, installed on this thread.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self::with_grants(Grants::none())
    }

    /// A fake host with these grants and no declared settings or panels.
    pub fn with_grants(grants: Grants) -> Self {
        Self::install(Declared {
            grants,
            settings: BTreeMap::new(),
            panels: BTreeSet::new(),
        })
    }

    /// A fake host with the grants, settings (with their defaults) and
    /// panels a `plugin.toml` declares. Panics on a manifest it cannot read.
    pub fn from_manifest(src: &str) -> Self {
        Self::install(parse_manifest(src).unwrap_or_else(|e| panic!("{e}")))
    }

    fn install(d: Declared) -> Self {
        let state = Rc::new(RefCell::new(State::new(d)));
        CURRENT.with(|c| *c.borrow_mut() = Some(state.clone()));
        Self { state }
    }

    fn get<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        f(&self.state.borrow())
    }

    fn set(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.state.borrow_mut())
    }

    // Setup.

    pub fn set_grants(&self, grants: Grants) {
        self.set(|s| s.grants = grants);
    }
    pub fn set_invocation(&self, invocation: Invocation) {
        self.set(|s| s.invocation = invocation);
    }
    /// Dev mode keeps `debug` log lines.
    pub fn set_dev_mode(&self, on: bool) {
        self.set(|s| s.dev_mode = on);
    }
    /// Makes every call to `call` (`"<interface>.<function>"`, for example
    /// `"http.fetch"` or `"workspace.history"`) fail with `error` until
    /// [`FakeHost::clear_injections`].
    pub fn inject(&self, call: &str, error: Error) {
        self.set(|s| {
            s.injections.insert(call.into(), (error, false));
        });
    }
    /// As [`FakeHost::inject`], for the next call only.
    pub fn inject_once(&self, call: &str, error: Error) {
        self.set(|s| {
            s.injections.insert(call.into(), (error, true));
        });
    }
    pub fn clear_injections(&self) {
        self.set(|s| s.injections.clear());
    }
    pub fn set_app_info(&self, info: AppInfo) {
        self.set(|s| s.app_info = info);
    }
    /// The wall clock `host.now` returns and rates use, in ms since the Unix
    /// epoch.
    pub fn set_now(&self, now: Timestamp) {
        self.set(|s| s.now = now);
    }
    /// Moves the clock forward.
    pub fn advance(&self, ms: i64) {
        self.set(|s| s.now += ms);
    }
    /// A plugin string for `host.t`; `{ $name }` placeholders are filled.
    pub fn set_string(&self, key: &str, value: &str) {
        self.set(|s| {
            s.strings.insert(key.into(), value.into());
        });
    }
    pub fn set_context(&self, context: Context) {
        self.set(|s| s.context = context);
    }
    /// Declares a setting (as the manifest would) with this value.
    pub fn set_setting(&self, key: &str, value: SettingValue) {
        self.set(|s| {
            s.settings.insert(key.into(), Some(value));
        });
    }
    /// The composer draft; without one, `composer` calls are `unsupported`.
    pub fn set_draft(&self, draft: &str) {
        self.set(|s| s.draft = Some(draft.into()));
    }
    /// A view the host assigned, such as the `view` passed to `render` or
    /// `run-slash`, so the plugin may update it.
    pub fn add_view(&self, view: &str) {
        self.set(|s| {
            s.views.insert(
                view.into(),
                OpenView {
                    kind: ViewKind::Host,
                    tree: None,
                },
            );
        });
    }
    /// Fires a pending `refresh-after` for `view`, as the host's timer would;
    /// false if none was pending. The test then calls the plugin's
    /// `view_event` with `Event::Refresh`.
    pub fn fire_refresh(&self, view: &str) -> bool {
        self.state.borrow_mut().refreshes.remove(view)
    }
    pub fn on_http(&self, handler: impl FnMut(&Request) -> Result<Response, Error> + 'static) {
        self.set(|s| s.http = Some(Box::new(handler)));
    }
    pub fn on_exec(
        &self,
        handler: impl FnMut(&str, &[String]) -> Result<ExecOutput, Error> + 'static,
    ) {
        self.set(|s| s.exec = Some(Box::new(handler)));
    }

    // Reading back.

    pub fn logs(&self) -> Vec<(LogLevel, String)> {
        self.get(|s| s.logs.clone())
    }
    pub fn ui_calls(&self) -> Vec<UiCall> {
        self.get(|s| s.ui.clone())
    }
    pub fn toasts(&self) -> Vec<(String, ToastKind)> {
        self.get(|s| {
            s.ui.iter()
                .filter_map(|c| match c {
                    UiCall::Toast { text, kind } => Some((text.clone(), *kind)),
                    _ => None,
                })
                .collect()
        })
    }
    /// The current tree of an open view, after updates and patches.
    pub fn view_tree(&self, view: &str) -> Option<ViewTree> {
        self.get(|s| s.views.get(view).and_then(|v| v.tree.clone()))
    }
    pub fn writes(&self) -> Vec<WriteCall> {
        self.get(|s| s.writes.clone())
    }
    /// The `send` and `reply` writes.
    pub fn sent(&self) -> Vec<Sent> {
        self.get(|s| {
            s.writes
                .iter()
                .filter_map(|w| match w {
                    WriteCall::Send(m) | WriteCall::Reply(m) => Some(m.clone()),
                    _ => None,
                })
                .collect()
        })
    }
    pub fn draft(&self) -> Option<String> {
        self.get(|s| s.draft.clone())
    }
    pub fn http_requests(&self) -> Vec<Request> {
        self.get(|s| s.http_requests.clone())
    }
    pub fn storage_value(&self, scope: &Scope, key: &str) -> Option<Vec<u8>> {
        self.get(|s| s.storage.get(&(scope_key(scope), key.to_string())).cloned())
    }
}

impl Drop for FakeHost {
    fn drop(&mut self) {
        CURRENT.with(|c| {
            let mut c = c.borrow_mut();
            if c.as_ref().is_some_and(|s| Rc::ptr_eq(s, &self.state)) {
                *c = None;
            }
        });
    }
}

/// The fake implementations `crate::host` forwards to.
#[doc(hidden)]
pub mod imp {
    use super::*;

    pub mod host {
        use super::*;
        pub fn api_version() -> Result<Version, Error> {
            gate("host.api_version")?;
            Ok(Version {
                major: 0,
                minor: 1,
                patch: 0,
            })
        }
        pub fn get_app_info() -> Result<AppInfo, Error> {
            gate("host.get_app_info")?;
            Ok(with(|s| s.app_info.clone()))
        }
        /// Cuts at 4 KB on a character boundary; `debug` only in dev mode.
        pub fn log(level: LogLevel, message: &str) -> Result<(), Error> {
            gate("host.log")?;
            let mut end = message.len().min(4096);
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            with(|s| {
                if level != LogLevel::Debug || s.dev_mode {
                    s.logs.push((level, message[..end].into()));
                }
            });
            Ok(())
        }
        pub fn now() -> Result<Timestamp, Error> {
            gate("host.now")?;
            Ok(with(|s| s.now))
        }
        /// A missing key returns the key itself.
        pub fn t(key: &str, args: &[(&str, &str)]) -> Result<String, Error> {
            gate("host.t")?;
            too_big("t key", key.len(), 128)?;
            if args.len() > 16 {
                return Err(Error::Limit(format!(
                    "t: {} arguments, at most 16",
                    args.len()
                )));
            }
            let Some(mut text) = with(|s| s.strings.get(key).cloned()) else {
                return Ok(key.into());
            };
            for (name, value) in args {
                text = text
                    .replace(&format!("{{ ${name} }}"), value)
                    .replace(&format!("{{${name}}}"), value);
            }
            Ok(text)
        }
    }

    pub mod workspace {
        pub use super::super::reads_imp::*;
        use super::*;

        pub fn get_context() -> Result<Context, Error> {
            gate("workspace.get_context")?;
            Ok(with(|s| s.context.clone()))
        }

        /// Injection, event hook, area, argument check, rate, then record.
        fn write<R>(
            call: &str,
            area: &str,
            check: Result<(), Error>,
            record: WriteCall,
            out: R,
        ) -> Result<R, Error> {
            gate(call)?;
            if with(|s| s.invocation == Invocation::EventHook) {
                return Err(Error::Denied(format!("{area} inside an event hook")));
            }
            if !with(|s| s.grants.workspace_write.contains(area)) {
                return Err(denied(area));
            }
            check?;
            rate("workspace.write", 20)?;
            with(|s| s.writes.push(record));
            Ok(out)
        }

        fn next_id(prefix: &str) -> String {
            with(|s| {
                s.next_id += 1;
                format!("{prefix}-{}", s.next_id)
            })
        }

        fn emoji(e: &str) -> Result<(), Error> {
            too_big("emoji", e.len(), 100)
        }

        pub fn send(ws: &str, conversation: &str, content: &Content) -> Result<MessageRef, Error> {
            let sent = Sent {
                workspace: ws.into(),
                conversation: conversation.into(),
                thread: None,
                content: content.clone(),
                also_to_channel: false,
            };
            let out = MessageRef {
                conversation: conversation.into(),
                message: next_id("message"),
            };
            write(
                "workspace.send",
                "messages:write",
                content_ok(content),
                WriteCall::Send(sent),
                out,
            )
        }
        pub fn reply(
            ws: &str,
            conversation: &str,
            thread: &str,
            content: &Content,
            also_to_channel: bool,
        ) -> Result<MessageRef, Error> {
            let sent = Sent {
                workspace: ws.into(),
                conversation: conversation.into(),
                thread: Some(thread.into()),
                content: content.clone(),
                also_to_channel,
            };
            let out = MessageRef {
                conversation: conversation.into(),
                message: next_id("message"),
            };
            write(
                "workspace.reply",
                "messages:write",
                content_ok(content),
                WriteCall::Reply(sent),
                out,
            )
        }
        pub fn edit(ws: &str, message: &MessageRef, content: &Content) -> Result<(), Error> {
            let rec = WriteCall::Edit {
                workspace: ws.into(),
                message: message.clone(),
                content: content.clone(),
            };
            write(
                "workspace.edit",
                "messages:edit",
                content_ok(content),
                rec,
                (),
            )
        }
        pub fn delete(ws: &str, message: &MessageRef) -> Result<(), Error> {
            let rec = WriteCall::Delete {
                workspace: ws.into(),
                message: message.clone(),
            };
            write("workspace.delete", "messages:edit", Ok(()), rec, ())
        }
        pub fn react(ws: &str, message: &MessageRef, e: &str) -> Result<(), Error> {
            let rec = WriteCall::React {
                workspace: ws.into(),
                message: message.clone(),
                emoji: e.into(),
            };
            write("workspace.react", "reactions:write", emoji(e), rec, ())
        }
        pub fn unreact(ws: &str, message: &MessageRef, e: &str) -> Result<(), Error> {
            let rec = WriteCall::Unreact {
                workspace: ws.into(),
                message: message.clone(),
                emoji: e.into(),
            };
            write("workspace.unreact", "reactions:write", emoji(e), rec, ())
        }
        pub fn add_reminder(ws: &str, text: &str, at: Timestamp) -> Result<ReminderId, Error> {
            let rec = WriteCall::AddReminder {
                workspace: ws.into(),
                text: text.into(),
                at,
            };
            let id = next_id("reminder");
            write(
                "workspace.add_reminder",
                "reminders:write",
                too_big("reminder text", text.len(), 4096),
                rec,
                id,
            )
        }
        pub fn complete_reminder(ws: &str, reminder: &str) -> Result<(), Error> {
            let rec = WriteCall::CompleteReminder {
                workspace: ws.into(),
                reminder: reminder.into(),
            };
            write(
                "workspace.complete_reminder",
                "reminders:write",
                Ok(()),
                rec,
                (),
            )
        }
        pub fn delete_reminder(ws: &str, reminder: &str) -> Result<(), Error> {
            let rec = WriteCall::DeleteReminder {
                workspace: ws.into(),
                reminder: reminder.into(),
            };
            write(
                "workspace.delete_reminder",
                "reminders:write",
                Ok(()),
                rec,
                (),
            )
        }
        pub fn schedule(
            ws: &str,
            conversation: &str,
            content: &Content,
            at: Timestamp,
        ) -> Result<ScheduledId, Error> {
            let rec = WriteCall::Schedule {
                workspace: ws.into(),
                conversation: conversation.into(),
                content: content.clone(),
                at,
            };
            let id = next_id("scheduled");
            write(
                "workspace.schedule",
                "scheduled:write",
                content_ok(content),
                rec,
                id,
            )
        }
        pub fn cancel_scheduled(ws: &str, scheduled: &str) -> Result<(), Error> {
            let rec = WriteCall::CancelScheduled {
                workspace: ws.into(),
                scheduled: scheduled.into(),
            };
            write(
                "workspace.cancel_scheduled",
                "scheduled:write",
                Ok(()),
                rec,
                (),
            )
        }
        pub fn upload(
            ws: &str,
            conversation: &str,
            name: &str,
            bytes: &[u8],
            comment: Option<&str>,
        ) -> Result<File, Error> {
            let check = too_big("upload", bytes.len(), 8 * MIB)
                .and_then(|_| too_big("upload name", name.len(), 255))
                .and_then(|_| too_big("upload comment", comment.map_or(0, str::len), 4096));
            let rec = WriteCall::Upload {
                workspace: ws.into(),
                conversation: conversation.into(),
                name: name.into(),
                bytes: bytes.to_vec(),
                comment: comment.map(Into::into),
            };
            let file = File {
                id: next_id("file"),
                name: name.into(),
                mime_type: "application/octet-stream".into(),
                size: bytes.len() as u64,
                created: with(|s| s.now),
            };
            write("workspace.upload", "files:write", check, rec, file)
        }
        pub fn set_status(ws: &str, status: &Status) -> Result<(), Error> {
            let rec = WriteCall::SetStatus {
                workspace: ws.into(),
                status: status.clone(),
            };
            write(
                "workspace.set_status",
                "status:write",
                too_big("status text", status.text.len(), 100),
                rec,
                (),
            )
        }
        pub fn set_presence(ws: &str, presence: Presence) -> Result<(), Error> {
            let check = if presence == Presence::Unknown {
                Err(Error::Invalid("presence `unknown` cannot be set".into()))
            } else {
                Ok(())
            };
            let rec = WriteCall::SetPresence {
                workspace: ws.into(),
                presence,
            };
            write("workspace.set_presence", "status:write", check, rec, ())
        }
        pub fn snooze(ws: &str, minutes: u32) -> Result<(), Error> {
            let check = if (1..=1440).contains(&minutes) {
                Ok(())
            } else {
                Err(Error::Invalid(format!(
                    "snooze {minutes} minutes not in 1 to 1,440"
                )))
            };
            let rec = WriteCall::Snooze {
                workspace: ws.into(),
                minutes,
            };
            write("workspace.snooze", "status:write", check, rec, ())
        }
        pub fn end_snooze(ws: &str) -> Result<(), Error> {
            write(
                "workspace.end_snooze",
                "status:write",
                Ok(()),
                WriteCall::EndSnooze {
                    workspace: ws.into(),
                },
                (),
            )
        }
        pub fn set_saved(ws: &str, message: &MessageRef, saved: bool) -> Result<(), Error> {
            let rec = WriteCall::SetSaved {
                workspace: ws.into(),
                message: message.clone(),
                saved,
            };
            write("workspace.set_saved", "saved:write", Ok(()), rec, ())
        }
        pub fn set_pinned(ws: &str, message: &MessageRef, pinned: bool) -> Result<(), Error> {
            let rec = WriteCall::SetPinned {
                workspace: ws.into(),
                message: message.clone(),
                pinned,
            };
            write("workspace.set_pinned", "pins:write", Ok(()), rec, ())
        }
        pub fn mark_read(ws: &str, conversation: &str, upto: &str) -> Result<(), Error> {
            let rec = WriteCall::MarkRead {
                workspace: ws.into(),
                conversation: conversation.into(),
                upto: upto.into(),
            };
            write("workspace.mark_read", "read_state:write", Ok(()), rec, ())
        }
    }

    pub mod http {
        use super::*;
        /// Header names the host sets or refuses (WIT `http`).
        fn forbidden(name: &str) -> bool {
            let n = name.to_ascii_lowercase();
            matches!(n.as_str(), "host" | "cookie" | "user-agent") || n.starts_with("proxy-")
        }
        pub fn fetch(request: &Request) -> Result<Response, Error> {
            gate("http.fetch")?;
            if !request.url.starts_with("https://") {
                return Err(Error::Invalid("http takes https URLs only".into()));
            }
            too_big("url", request.url.len(), 8 * 1024)?;
            granted_host(&request.url)?;
            if request.headers.len() > 32 {
                return Err(Error::Limit(format!(
                    "{} headers, at most 32",
                    request.headers.len()
                )));
            }
            for (name, value) in &request.headers {
                too_big("header", name.len() + value.len(), 8 * 1024)?;
                if forbidden(name) {
                    return Err(Error::Invalid(format!(
                        "header `{name}` is set by the host or refused"
                    )));
                }
            }
            too_big("body", request.body.as_ref().map_or(0, Vec::len), MIB)?;
            if request.timeout_ms > 30_000 {
                return Err(Error::Limit(format!(
                    "timeout {} ms, at most 30,000",
                    request.timeout_ms
                )));
            }
            rate("http.fetch", 120)?;
            let st = state();
            let handler = {
                let mut s = st.borrow_mut();
                s.http_requests.push(request.clone());
                s.http.take()
            };
            let Some(mut handler) = handler else {
                return Err(Error::Offline);
            };
            let out = handler(request);
            st.borrow_mut().http = Some(handler);
            let response = out?;
            too_big(
                "response body",
                response.body.len(),
                (4 * MIB).min(quarter_memory()),
            )?;
            Ok(response)
        }
    }

    pub mod storage {
        use super::*;
        fn granted() -> Result<(), Error> {
            if with(|s| s.grants.storage) {
                Ok(())
            } else {
                Err(denied("storage"))
            }
        }
        fn key_ok(key: &str) -> Result<(), Error> {
            too_big("storage key", key.len(), 256)
        }
        pub fn get(scope: &Scope, key: &str) -> Result<Option<Vec<u8>>, Error> {
            gate("storage.get")?;
            granted()?;
            key_ok(key)?;
            Ok(with(|s| {
                s.storage.get(&(scope_key(scope), key.to_string())).cloned()
            }))
        }
        /// At most 256 KB a value, and 5 MB (keys plus values) and 2,000
        /// keys per plugin, across scopes.
        pub fn set(scope: &Scope, key: &str, value: &[u8]) -> Result<(), Error> {
            gate("storage.set")?;
            granted()?;
            key_ok(key)?;
            too_big("storage value", value.len(), 256 * 1024)?;
            with(|s| {
                let k = (scope_key(scope), key.to_string());
                let old = s.storage.get(&k).map(|v| k.1.len() + v.len());
                let total: usize = s.storage.iter().map(|((_, k), v)| k.len() + v.len()).sum();
                let total = total - old.unwrap_or(0) + key.len() + value.len();
                if total > 5 * MIB {
                    return Err(Error::Limit(format!(
                        "storage: {total} bytes, at most 5 MB"
                    )));
                }
                if old.is_none() && s.storage.len() >= 2_000 {
                    return Err(Error::Limit("storage: at most 2,000 keys".into()));
                }
                s.storage.insert(k, value.to_vec());
                Ok(())
            })
        }
        pub fn delete(scope: &Scope, key: &str) -> Result<(), Error> {
            gate("storage.delete")?;
            granted()?;
            key_ok(key)?;
            with(|s| s.storage.remove(&(scope_key(scope), key.to_string())));
            Ok(())
        }
        /// Keys in order; the cursor is the last key of the previous page.
        pub fn keys(
            scope: &Scope,
            prefix: &str,
            cursor: Option<&str>,
            limit: u32,
        ) -> Result<KeyPage, Error> {
            gate("storage.keys")?;
            granted()?;
            page_limit(limit)?;
            let scope = scope_key(scope);
            with(|s| {
                let mut keys = s
                    .storage
                    .keys()
                    .filter(|(sc, k)| *sc == scope && k.starts_with(prefix))
                    .map(|(_, k)| k.clone())
                    .filter(|k| cursor.is_none_or(|c| k.as_str() > c));
                let items: Vec<String> = keys.by_ref().take(limit as usize).collect();
                let next = if keys.next().is_some() {
                    items.last().cloned()
                } else {
                    None
                };
                Ok(KeyPage { items, next })
            })
        }
        pub fn clear(scope: &Scope) -> Result<(), Error> {
            gate("storage.clear")?;
            granted()?;
            let scope = scope_key(scope);
            with(|s| s.storage.retain(|(sc, _), _| *sc != scope));
            Ok(())
        }
    }

    pub mod secrets {
        use super::*;
        pub fn secret_get(key: &str) -> Result<Option<String>, Error> {
            gate("secrets.secret_get")?;
            too_big("secret key", key.len(), 128)?;
            Ok(with(|s| s.secrets.get(key).cloned()))
        }
        /// At most 32 keys, keys 128 bytes and values 4 KB.
        pub fn secret_set(key: &str, value: &str) -> Result<(), Error> {
            gate("secrets.secret_set")?;
            too_big("secret key", key.len(), 128)?;
            too_big("secret value", value.len(), 4096)?;
            with(|s| {
                if !s.secrets.contains_key(key) && s.secrets.len() >= 32 {
                    return Err(Error::Limit("secrets: at most 32 keys".into()));
                }
                s.secrets.insert(key.into(), value.into());
                Ok(())
            })
        }
        pub fn secret_delete(key: &str) -> Result<(), Error> {
            gate("secrets.secret_delete")?;
            too_big("secret key", key.len(), 128)?;
            with(|s| s.secrets.remove(key));
            Ok(())
        }
    }

    pub mod settings {
        use super::*;
        /// Only declared keys (from the manifest or `FakeHost::set_setting`)
        /// exist; others are `not-found`.
        pub fn setting(key: &str) -> Result<Option<SettingValue>, Error> {
            gate("settings.setting")?;
            with(|s| s.settings.get(key).cloned()).ok_or(Error::NotFound)
        }
    }

    pub mod ui {
        use super::*;

        fn push(call: UiCall) {
            with(|s| s.ui.push(call));
        }
        fn new_view(kind: ViewKind, tree: &ViewTree) -> ViewId {
            with(|s| {
                s.next_id += 1;
                let id = format!("view-{}", s.next_id);
                s.views.insert(
                    id.clone(),
                    OpenView {
                        kind,
                        tree: Some(tree.clone()),
                    },
                );
                id
            })
        }
        fn open(view: &str) -> Result<(), Error> {
            if with(|s| s.views.contains_key(view)) {
                Ok(())
            } else {
                Err(Error::NotFound)
            }
        }
        fn count(kind: ViewKind) -> usize {
            with(|s| s.views.values().filter(|v| v.kind == kind).count())
        }

        /// One panel open at a time (the fake has one window).
        pub fn open_panel(panel_id: &str, tree: &ViewTree) -> Result<ViewId, Error> {
            gate("ui.open_panel")?;
            if !with(|s| s.grants.panel) {
                return Err(denied("panel"));
            }
            if with(|s| !s.panels.is_empty() && !s.panels.contains(panel_id)) {
                return Err(Error::NotFound);
            }
            if count(ViewKind::Panel) >= 1 {
                return Err(Error::Limit("panel: one per plugin per window".into()));
            }
            check_tree(tree)?;
            let view = new_view(ViewKind::Panel, tree);
            push(UiCall::OpenPanel {
                panel: panel_id.into(),
                view: view.clone(),
                tree: tree.clone(),
            });
            Ok(view)
        }
        pub fn open_window(spec: &WindowSpec, tree: &ViewTree) -> Result<ViewId, Error> {
            gate("ui.open_window")?;
            if !with(|s| s.grants.windows) {
                return Err(denied("windows"));
            }
            too_big("window title", spec.title.len(), 100)?;
            if count(ViewKind::Window) >= 2 {
                return Err(Error::Limit("windows: at most 2 per plugin".into()));
            }
            check_tree(tree)?;
            let view = new_view(ViewKind::Window, tree);
            push(UiCall::OpenWindow {
                spec: spec.clone(),
                view: view.clone(),
                tree: tree.clone(),
            });
            Ok(view)
        }
        pub fn update(view: &str, tree: &ViewTree) -> Result<(), Error> {
            gate("ui.update")?;
            open(view)?;
            check_tree(tree)?;
            with(|s| s.views.get_mut(view).unwrap().tree = Some(tree.clone()));
            push(UiCall::Update {
                view: view.into(),
                tree: tree.clone(),
            });
            Ok(())
        }
        /// The patched view is checked whole, so ids stay unique across it.
        pub fn patch(view: &str, node: &str, tree: &ViewTree) -> Result<(), Error> {
            gate("ui.patch")?;
            open(view)?;
            check_tree(tree)?;
            let base = with(|s| s.views[view].tree.clone());
            let next = match base {
                Some(base) => Some(view::apply_patch(&base, node, tree)?),
                None => None,
            };
            if let Some(next) = next {
                with(|s| s.views.get_mut(view).unwrap().tree = Some(next));
            }
            push(UiCall::Patch {
                view: view.into(),
                node: node.into(),
                tree: tree.clone(),
            });
            Ok(())
        }
        pub fn close(view: &str) -> Result<(), Error> {
            gate("ui.close")?;
            open(view)?;
            with(|s| {
                s.views.remove(view);
                s.refreshes.remove(view);
            });
            push(UiCall::Close { view: view.into() });
            Ok(())
        }
        /// At least 5 s; one pending request per view and 4 per plugin.
        pub fn refresh_after(view: &str, seconds: u32) -> Result<(), Error> {
            gate("ui.refresh_after")?;
            open(view)?;
            if seconds < 5 {
                return Err(Error::Invalid("refresh-after is at least 5 s".into()));
            }
            with(|s| {
                if s.refreshes.contains(view) {
                    return Err(Error::Limit(
                        "refresh-after: one pending request per view".into(),
                    ));
                }
                if s.refreshes.len() >= 4 {
                    return Err(Error::Limit(
                        "refresh-after: 4 pending requests per plugin".into(),
                    ));
                }
                s.refreshes.insert(view.into());
                Ok(())
            })?;
            push(UiCall::RefreshAfter {
                view: view.into(),
                seconds,
            });
            Ok(())
        }
        /// At most 200 bytes and 6 per minute; while handling a user action
        /// or with a view open.
        pub fn toast(text: &str, kind: ToastKind) -> Result<(), Error> {
            gate("ui.toast")?;
            too_big("toast", text.len(), 200)?;
            if with(|s| s.invocation != Invocation::UserAction && s.views.is_empty()) {
                return Err(Error::Denied(
                    "toast only while handling a user action or with a view open".into(),
                ));
            }
            rate("ui.toast", 6)?;
            push(UiCall::Toast {
                text: text.into(),
                kind,
            });
            Ok(())
        }
        pub fn notify(title: &str, body: &str, target: &NotifyTarget) -> Result<(), Error> {
            gate("ui.notify")?;
            if !with(|s| s.grants.notifications) {
                return Err(denied("notifications"));
            }
            too_big("notify title", title.len(), 100)?;
            too_big("notify body", body.len(), 500)?;
            push(UiCall::Notify {
                title: title.into(),
                body: body.into(),
                target: target.clone(),
            });
            Ok(())
        }
        pub fn copy(text: &str) -> Result<(), Error> {
            gate("ui.copy")?;
            if !with(|s| s.grants.clipboard_write) {
                return Err(denied("clipboard_write"));
            }
            user_action("copy")?;
            too_big("copy", text.len(), MIB)?;
            push(UiCall::Copy { text: text.into() });
            Ok(())
        }
        pub fn open_url(url: &str) -> Result<(), Error> {
            gate("ui.open_url")?;
            user_action("open-url")?;
            if https_host(url).is_none() {
                return Err(Error::Invalid("open-url takes https only".into()));
            }
            push(UiCall::OpenUrl { url: url.into() });
            Ok(())
        }
        pub fn open_conversation(
            ws: &str,
            conversation: &str,
            message: Option<&str>,
        ) -> Result<(), Error> {
            gate("ui.open_conversation")?;
            user_action("open-conversation")?;
            push(UiCall::OpenConversation {
                workspace: ws.into(),
                conversation: conversation.into(),
                message: message.map(Into::into),
            });
            Ok(())
        }
    }

    pub mod composer {
        use super::*;
        const MAX: usize = 40 * 1024;
        fn edit(call: &str, f: impl FnOnce(&mut String)) -> Result<(), Error> {
            gate(call)?;
            with(|s| {
                let Some(d) = &mut s.draft else {
                    return Err(Error::Unsupported);
                };
                let mut next = d.clone();
                f(&mut next);
                too_big("draft", next.len(), MAX)?;
                *d = next;
                Ok(())
            })
        }
        pub fn draft() -> Result<String, Error> {
            gate("composer.draft")?;
            with(|s| s.draft.clone()).ok_or(Error::Unsupported)
        }
        pub fn replace_draft(text: &str) -> Result<(), Error> {
            edit("composer.replace_draft", |d| *d = text.into())
        }
        /// The fake's cursor is at the end of the draft.
        pub fn insert(text: &str) -> Result<(), Error> {
            edit("composer.insert", |d| d.push_str(text))
        }
    }

    pub mod exec {
        use super::*;
        /// The program and argv must match a granted shape; at most 32
        /// arguments of 1 KB and 30 runs per minute.
        pub fn run(program: &str, args: &[String]) -> Result<ExecOutput, Error> {
            gate("exec.run")?;
            if args.len() > 32 {
                return Err(Error::Limit(format!(
                    "exec: {} arguments, at most 32",
                    args.len()
                )));
            }
            for a in args {
                too_big("exec argument", a.len(), 1024)?;
            }
            let allowed = with(|s| {
                s.grants
                    .exec
                    .iter()
                    .filter(|g| g.program == program)
                    .any(|g| {
                        g.allow.iter().any(|shape| {
                            shape.len() == args.len()
                                && shape.iter().zip(args).all(|(p, a)| glob(p, a))
                        })
                    })
            });
            if !allowed {
                return Err(Error::Denied(format!(
                    "exec: {program} with these arguments"
                )));
            }
            rate("exec.run", 30)?;
            let st = state();
            let handler = st.borrow_mut().exec.take();
            let Some(mut handler) = handler else {
                return Err(Error::Failed("no exec handler set".into()));
            };
            let out = handler(program, args);
            st.borrow_mut().exec = Some(handler);
            out
        }
    }
}
