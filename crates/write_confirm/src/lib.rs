//! The write confirmation seam and write rate limiter shared by MCP and
//! plugins. A write that
//! needs the user's yes is described to a [`Confirmer`], which the app
//! implements with its confirmation window; the caller applies
//! [`CONFIRM_TIMEOUT`] around it. Neutral: no GPUI, no provider.

pub mod action;
pub mod limits;

use std::{sync::Arc, time::Duration};

use domain::ConversationKind;
use futures::future::{BoxFuture, ready};
use settings::WorkspaceKey;

pub use action::{ReminderKind, WriteAction};
pub use limits::{ClientId, RequesterKey};

/// How long the caller waits for the user before treating a write as declined.
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(120);

/// The MCP client making a call, as the audit log, the limits and the prompt
/// know it. The name is client-supplied and only a label.
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub id: ClientId,
    pub name: Arc<str>,
    pub peer_pid: Option<u32>,
}

/// Where an installed plugin came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginSource {
    Registry,
    File,
    Dev,
    BuiltIn,
}

/// Who asks for the write.
#[derive(Debug, Clone)]
pub enum Requester {
    Mcp(ClientInfo),
    Plugin {
        id: Arc<str>,
        name: Arc<str>,
        source: PluginSource,
    },
}

impl Requester {
    /// The name the prompt shows: the client's or the plugin's.
    pub fn name(&self) -> &str {
        match self {
            Requester::Mcp(c) => &c.name,
            Requester::Plugin { name, .. } => name,
        }
    }

    /// The limiter key.
    pub fn key(&self) -> RequesterKey {
        match self {
            Requester::Mcp(c) => RequesterKey::Mcp(c.id),
            Requester::Plugin { id, .. } => RequesterKey::Plugin(id.clone()),
        }
    }

    pub fn is_plugin(&self) -> bool {
        matches!(self, Requester::Plugin { .. })
    }
}

impl From<&Requester> for RequesterKey {
    fn from(r: &Requester) -> Self {
        r.key()
    }
}

/// The workspace a write goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub key: WorkspaceKey,
    pub name: Arc<str>,
}

/// What the prompt offers to remember with a yes (plugins only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowOffer {
    None,
    /// "Allow without asking in #name": messages, edits, files, scheduled.
    InConversation(Arc<str>),
    /// "Allow without asking for this plugin": the other areas.
    ForPlugin,
}

impl AllowOffer {
    /// The offer a plugin write of `action` gets: per conversation for
    /// messages, edits, files and scheduled messages, per plugin for the
    /// rest, and never for a delete (it always asks).
    pub fn for_action(action: &WriteAction) -> Self {
        match action {
            WriteAction::Delete { .. } => AllowOffer::None,
            WriteAction::Send {
                conversation_name, ..
            }
            | WriteAction::Schedule {
                conversation_name, ..
            }
            | WriteAction::Edit {
                conversation_name, ..
            }
            | WriteAction::Upload {
                conversation_name, ..
            } => AllowOffer::InConversation(conversation_name.clone()),
            _ => AllowOffer::ForPlugin,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WriteRequest {
    pub requester: Requester,
    pub workspace: Workspace,
    /// The display name of the user the write is made as, when known.
    pub as_user: Option<Arc<str>>,
    pub action: WriteAction,
    pub offer: AllowOffer,
    /// The kind of the destination conversation, when known; the prompt
    /// picks its sigil from it (`None` draws `#`, as MCP always has).
    pub conversation_kind: Option<ConversationKind>,
}

impl WriteRequest {
    /// A request with the offer its requester and action get: none for MCP,
    /// [`AllowOffer::for_action`] for a plugin.
    pub fn new(
        requester: Requester,
        workspace: Workspace,
        as_user: Option<Arc<str>>,
        action: WriteAction,
    ) -> Self {
        let offer = if requester.is_plugin() {
            AllowOffer::for_action(&action)
        } else {
            AllowOffer::None
        };
        Self {
            requester,
            workspace,
            as_user,
            action,
            offer,
            conversation_kind: None,
        }
    }

    /// The same request with the destination conversation's kind.
    pub fn with_conversation_kind(mut self, kind: ConversationKind) -> Self {
        self.conversation_kind = Some(kind);
        self
    }

    /// The offer the prompt shows: never for MCP, never for a delete.
    pub fn remember_offer(&self) -> &AllowOffer {
        const NONE: &AllowOffer = &AllowOffer::None;
        match (&self.requester, &self.action) {
            (Requester::Mcp(_), _) | (_, WriteAction::Delete { .. }) => NONE,
            _ => &self.offer,
        }
    }

    /// The write as it was asked for, as pretty JSON (the prompt's Details
    /// disclosure). Names stand in for ids, like the rest of the prompt;
    /// message text is carried exactly.
    pub fn details(&self) -> String {
        use serde_json::{Value, json};
        let mut v = match &self.requester {
            Requester::Mcp(_) => json!({
                "tool": self.action.tool(),
                "workspace": self.workspace.key.to_string(),
            }),
            Requester::Plugin { id, .. } => json!({
                "plugin": &**id,
                "action": self.action.tool(),
                "workspace": self.workspace.key.to_string(),
            }),
        };
        if let (Some(o), Value::Object(a)) = (v.as_object_mut(), self.action.args()) {
            o.extend(a);
        }
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    /// Allowed. `remember`: also later writes without asking (for MCP, this
    /// tool to this destination for the session; for a plugin, the offer).
    Approved {
        remember: bool,
    },
    Declined,
    TimedOut,
}

/// Asks the user to confirm a write. The future may stay pending for a long
/// time; the caller drops it at [`CONFIRM_TIMEOUT`].
pub trait Confirmer: Send + Sync + 'static {
    fn confirm(&self, req: WriteRequest) -> BoxFuture<'static, Confirmation>;
}

/// Declines every write; for tests and for when no UI exists.
pub struct DenyAllConfirmer;

impl Confirmer for DenyAllConfirmer {
    fn confirm(&self, _req: WriteRequest) -> BoxFuture<'static, Confirmation> {
        Box::pin(ready(Confirmation::Declined))
    }
}
