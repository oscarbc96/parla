//! The check every host call goes through: the plugin must be enabled for the workspace, the grant must hold
//! the capability, a read must be in context unless the grant says
//! otherwise, gestures need a user action, views must be the plugin's own,
//! and the provider must support the area.

use std::fmt;
use std::path::PathBuf;

use domain::{Capabilities, ConversationId, ConversationKind, MessageTs};
use settings::WorkspaceKey;
use url::Url;

use super::grant::Grant;
use super::map::{area_capability, search_capability};
use super::net::{BadUrl, https_host};
use crate::manifest::{ReadArea, WriteArea};

/// A conversation a call targets or the context holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationRef {
    pub workspace: WorkspaceKey,
    pub id: ConversationId,
    pub kind: ConversationKind,
}

impl ConversationRef {
    fn same(&self, other: &Self) -> bool {
        self.workspace == other.workspace && self.id == other.id
    }

    fn is_direct(&self) -> bool {
        matches!(
            self.kind,
            ConversationKind::Direct | ConversationKind::Group
        )
    }
}

/// Messages of one conversation, or one message in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageTarget {
    pub conversation: ConversationRef,
    /// The one message read (`get-message`, `permalink`); `None` for a
    /// page (`history`, `replies`).
    pub message: Option<MessageTs>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchKind {
    Messages,
    Files,
    People,
}

/// The user's own lists (§9.2), each behind its `:read` area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserList {
    Reminders,
    Scheduled,
    Saved,
    Pins,
    Status,
}

impl UserList {
    pub fn area(self) -> ReadArea {
        match self {
            Self::Reminders => ReadArea::Reminders,
            Self::Scheduled => ReadArea::Scheduled,
            Self::Saved => ReadArea::Saved,
            Self::Pins => ReadArea::Pins,
            Self::Status => ReadArea::Status,
        }
    }
}

/// A view the host gave the plugin (panel, window, unfurl, slash note).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewId(pub u64);

/// One host function family (§9) with its target. [`HostCallKind`] lists
/// the families.
#[derive(Clone, Debug, PartialEq, Eq, strum::EnumDiscriminants)]
#[strum_discriminants(name(HostCallKind), derive(Hash, strum::EnumIter))]
pub enum HostCall {
    /// `host`: version, app info, log, now, `t`, and `get-context`.
    Host,
    /// `list-workspaces` (`None`), `me`.
    Workspace {
        workspace: Option<WorkspaceKey>,
    },
    /// `list-conversations`, `get-conversation`.
    Conversations {
        workspace: WorkspaceKey,
    },
    /// `history`, `replies`, `get-message`, `permalink`.
    Messages(MessageTarget),
    /// `get-user`, `find-users`, `get-presence`.
    People {
        workspace: WorkspaceKey,
    },
    /// `search-messages`, `search-files`, people search.
    Search {
        workspace: WorkspaceKey,
        kind: SearchKind,
    },
    /// `file-info`, `file-bytes`: files in a readable conversation.
    Files(MessageTarget),
    /// `list-reminders`, `list-scheduled`, `list-saved`, `list-pins`,
    /// `my-status`.
    List {
        workspace: WorkspaceKey,
        list: UserList,
    },
    /// Every `workspace` write (§9.3), by area.
    Write {
        workspace: WorkspaceKey,
        area: WriteArea,
    },
    /// `http.fetch`.
    Http {
        url: Url,
    },
    Storage,
    /// A plugin's own secrets; no capability.
    Secrets,
    /// A plugin's own settings; no capability.
    Settings,
    /// `ui.open-panel`.
    Panel,
    /// `ui.open-window`.
    Window,
    /// `ui.update`, `patch`, `close`, `refresh-after` on a view the plugin
    /// owns.
    View {
        view: ViewId,
    },
    /// `ui.toast`: while handling a user action or with a view open.
    Toast,
    /// `ui.notify`.
    Notify,
    /// `ui.copy`: while handling a user action.
    Copy,
    /// `ui.open-url`: https only, while handling a user click. The host
    /// asks first for a host not on the card.
    OpenUrl {
        url: Url,
    },
    /// `ui.open-conversation`: while handling a user click.
    OpenConversation,
    /// `composer.*`: while handling an invocation from a composer.
    Composer,
    /// `exec.run`, with where `program` resolves now.
    Exec {
        program: String,
        resolved: Option<PathBuf>,
        args: Vec<String>,
    },
}

impl HostCallKind {
    /// Every family, in declaration order.
    pub fn all() -> impl Iterator<Item = Self> {
        <Self as strum::IntoEnumIterator>::iter()
    }
}

/// What the plugin is handling when it calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A command, slash command, message action or view event the user
    /// triggered; `composer` when invoked from a composer.
    UserAction { composer: bool },
    /// No user action: a view refresh (`refresh-after`).
    View,
    /// An event hook (§10.5): hooks cannot write and were not invoked on a
    /// message.
    Hook,
    /// An MCP tool call (§10.11): not a user action in Parla.
    Mcp,
    /// Anything else: activation, a renderer, a filter, a transform.
    Other,
}

/// What the call happens in.
#[derive(Clone, Debug)]
pub struct CallContext<'a> {
    /// What the target workspace's provider supports.
    pub provider: &'a Capabilities,
    /// The conversation in context: shown in the window the plugin's view
    /// is in, or the one the user invoked the plugin on. Set only by the
    /// user (§7.3).
    pub conversation: Option<ConversationRef>,
    /// The message the user invoked the plugin on, in `conversation`.
    pub message: Option<MessageTs>,
    pub origin: Origin,
    /// The plugin's open views (a handful: at most 2 windows, one panel
    /// per window, visible unfurls).
    pub views: &'a [ViewId],
    /// The workspaces the plugin is enabled for; `None` is all of them.
    pub workspaces: Option<&'a [WorkspaceKey]>,
}

/// A capability a grant can lack; named in `error.denied(capability)`,
/// the plugin log and the toast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Need {
    Read(ReadArea),
    Write(WriteArea),
    /// The host the call tried to reach.
    Network(String),
    Storage,
    Windows,
    Panel,
    Notifications,
    ClipboardWrite,
    Exec(String),
}

impl fmt::Display for Need {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(a) => write!(f, "workspace_read: {a}"),
            Self::Write(a) => write!(f, "workspace_write: {a}"),
            Self::Network(h) => write!(f, "network: {h}"),
            Self::Storage => f.write_str("storage"),
            Self::Windows => f.write_str("windows"),
            Self::Panel => f.write_str("panel"),
            Self::Notifications => f.write_str("notifications"),
            Self::ClipboardWrite => f.write_str("clipboard_write"),
            Self::Exec(p) => write!(f, "exec: {p}"),
        }
    }
}

/// Why a call is refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Denied {
    /// Outside the grant: `error.denied`.
    #[error("not allowed: {0}")]
    Missing(Need),
    /// The provider lacks the feature: `error.unsupported`.
    #[error("not supported by this workspace")]
    Unsupported,
    /// Allowed only while handling a user action.
    #[error("only while handling a user action")]
    NeedsUserAction,
    /// Hooks cannot write (§7.5).
    #[error("event hooks cannot write")]
    HooksCannotWrite,
    /// The program resolves elsewhere than at consent, or was never
    /// resolved; the user is asked again (§7.4).
    #[error("the program is not the one granted")]
    ProgramMoved,
    /// `http` or `open-url` with a scheme other than https.
    #[error("only https links are allowed")]
    NotHttps,
    /// An `http` URL with a port, user info or an IP address.
    #[error("the URL may not carry a port, user info or an IP address")]
    InvalidUrl,
    /// A view call on a view the plugin does not own.
    #[error("not this plugin's view")]
    NotOwner,
    /// The plugin is not enabled for the target workspace.
    #[error("the plugin is not enabled for this workspace")]
    WorkspaceNotEnabled,
}

fn need(ok: bool, n: Need) -> Result<(), Denied> {
    if ok { Ok(()) } else { Err(Denied::Missing(n)) }
}

fn supported(ok: bool) -> Result<(), Denied> {
    if ok { Ok(()) } else { Err(Denied::Unsupported) }
}

fn user_action(cx: &CallContext<'_>) -> Result<(), Denied> {
    match cx.origin {
        Origin::UserAction { .. } => Ok(()),
        _ => Err(Denied::NeedsUserAction),
    }
}

/// A plain read area: granted, and the provider has it.
fn read(g: &Grant, cx: &CallContext<'_>, a: ReadArea) -> Result<(), Denied> {
    need(g.has_read(a), Need::Read(a))?;
    supported(area_capability(a.into())(cx.provider))
}

/// §7.3: `messages:read` covers the non-DM conversation in context, and in
/// a direct or group message only the message the user invoked the plugin
/// on; `history:read` covers any non-DM conversation; `dms:read` extends
/// both to direct and group messages.
fn readable(g: &Grant, t: &MessageTarget, cx: &CallContext<'_>) -> Result<(), Denied> {
    let conv = &t.conversation;
    let in_context = cx.conversation.as_ref().is_some_and(|c| c.same(conv));
    let in_context = g.has_read(ReadArea::Messages) && in_context;
    if conv.is_direct() {
        // A hook was not invoked on anything.
        let invoked = in_context
            && cx.origin != Origin::Hook
            && t.message.is_some()
            && t.message == cx.message;
        let extended = g.has_read(ReadArea::Dms) && (in_context || g.has_read(ReadArea::History));
        need(invoked || extended, Need::Read(ReadArea::Dms))
    } else if in_context || g.has_read(ReadArea::History) {
        Ok(())
    } else if cx.conversation.as_ref().is_some_and(|c| c.same(conv)) {
        Err(Denied::Missing(Need::Read(ReadArea::Messages)))
    } else {
        Err(Denied::Missing(Need::Read(ReadArea::History)))
    }
}

fn enabled(cx: &CallContext<'_>, ws: &WorkspaceKey) -> Result<(), Denied> {
    match cx.workspaces {
        Some(list) if !list.contains(ws) => Err(Denied::WorkspaceNotEnabled),
        _ => Ok(()),
    }
}

/// The workspace a call targets, if it targets one.
fn target_workspace(call: &HostCall) -> Option<&WorkspaceKey> {
    match call {
        HostCall::Workspace { workspace } => workspace.as_ref(),
        HostCall::Conversations { workspace }
        | HostCall::People { workspace }
        | HostCall::Search { workspace, .. }
        | HostCall::List { workspace, .. }
        | HostCall::Write { workspace, .. } => Some(workspace),
        HostCall::Messages(t) | HostCall::Files(t) => Some(&t.conversation.workspace),
        _ => None,
    }
}

/// Whether one item of a result (a search hit, a file hit, a hook event)
/// may reach the plugin: the same rules as reading it directly (§7.3).
/// The host drops the items this refuses before the plugin sees the page.
pub fn check_result(
    grant: &Grant,
    item: &MessageTarget,
    cx: &CallContext<'_>,
) -> Result<(), Denied> {
    enabled(cx, &item.conversation.workspace)?;
    readable(grant, item, cx)
}

/// Checks `call` against `grant` in `cx`, in this order:
///
/// 1. the plugin is enabled for the target workspace
///    (`WorkspaceNotEnabled`; `list-workspaces` has no target, the host
///    filters its answer);
/// 2. hooks cannot write (`HooksCannotWrite`);
/// 3. the call's own form: an `http` or `open-url` URL (`NotHttps`,
///    `InvalidUrl`);
/// 4. the grant (`Missing`), with the in-context rules for reads; for
///    `exec` the program is granted, then it resolves where it did at
///    consent (`ProgramMoved`), then a shape matches (`Missing`);
/// 5. the gesture and ownership rules (`NeedsUserAction`, `NotOwner`);
/// 6. the provider (`Unsupported`).
pub fn check(grant: &Grant, call: &HostCall, cx: &CallContext<'_>) -> Result<(), Denied> {
    let g = grant;
    if let Some(ws) = target_workspace(call) {
        enabled(cx, ws)?;
    }
    match call {
        HostCall::Host | HostCall::Secrets | HostCall::Settings => Ok(()),
        HostCall::Workspace { .. } => read(g, cx, ReadArea::Workspace),
        HostCall::Conversations { .. } => read(g, cx, ReadArea::Conversations),
        HostCall::Messages(t) => readable(g, t, cx),
        HostCall::People { .. } => read(g, cx, ReadArea::People),
        HostCall::Search { kind, .. } => {
            need(g.has_read(ReadArea::Search), Need::Read(ReadArea::Search))?;
            supported(search_capability(*kind)(cx.provider))
        }
        HostCall::Files(t) => {
            need(g.has_read(ReadArea::Files), Need::Read(ReadArea::Files))?;
            readable(g, t, cx)?;
            supported(area_capability(ReadArea::Files.into())(cx.provider))
        }
        HostCall::List { list, .. } => read(g, cx, list.area()),
        HostCall::Write { area, .. } => {
            if cx.origin == Origin::Hook {
                return Err(Denied::HooksCannotWrite);
            }
            need(g.has_write(*area), Need::Write(*area))?;
            supported(area_capability((*area).into())(cx.provider))
        }
        HostCall::Http { url } => {
            let host = https_host(url).map_err(|e| match e {
                BadUrl::NotHttps => Denied::NotHttps,
                BadUrl::Invalid => Denied::InvalidUrl,
            })?;
            need(
                g.network.iter().any(|h| h.matches(host)),
                Need::Network(host.trim_end_matches('.').to_owned()),
            )
        }
        HostCall::Storage => need(g.storage, Need::Storage),
        HostCall::Panel => need(g.panel, Need::Panel),
        HostCall::Window => need(g.windows, Need::Windows),
        HostCall::Notify => need(g.notifications, Need::Notifications),
        HostCall::View { view } => {
            if cx.views.contains(view) {
                Ok(())
            } else {
                Err(Denied::NotOwner)
            }
        }
        // §9.8: while handling a user action or with a view open, whatever
        // the plugin is handling (a hook or a renderer too).
        HostCall::Toast => match cx.origin {
            Origin::UserAction { .. } | Origin::View => Ok(()),
            _ if !cx.views.is_empty() => Ok(()),
            _ => Err(Denied::NeedsUserAction),
        },
        HostCall::Copy => {
            need(g.clipboard_write, Need::ClipboardWrite)?;
            user_action(cx)
        }
        HostCall::OpenUrl { url } => {
            if url.scheme() != "https" {
                return Err(Denied::NotHttps);
            }
            user_action(cx)
        }
        HostCall::OpenConversation => user_action(cx),
        HostCall::Composer => match cx.origin {
            Origin::UserAction { composer: true } => Ok(()),
            _ => Err(Denied::NeedsUserAction),
        },
        HostCall::Exec {
            program,
            resolved,
            args,
        } => {
            let rule = g
                .exec
                .iter()
                .find(|e| &e.program == program)
                .ok_or_else(|| Denied::Missing(Need::Exec(program.clone())))?;
            if rule.path.is_none() || rule.path != *resolved {
                return Err(Denied::ProgramMoved);
            }
            need(
                rule.allow.iter().any(|s| s.matches(args)),
                Need::Exec(program.clone()),
            )
        }
    }
}
