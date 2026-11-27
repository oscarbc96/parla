//! The policy gate: which workspaces and
//! conversations an MCP client may see, and whether it may write. Read from
//! the current settings snapshot on every call, so a settings edit applies to
//! the next call without reconnecting.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use domain::{Conversation, ConversationId, ConversationKind};
use settings::{McpSettings, McpWorkspaceSettings, WorkspaceKey};

/// Where the policy reads settings from. The app implements it over its
/// settings store; tests use a mutable fake.
pub trait SettingsSource: Send + Sync {
    fn mcp(&self) -> Arc<McpSettings>;
    fn include_content(&self) -> bool;
}

/// Why a request is refused. Tools report all of these to the agent as the
/// same "not available" error; the code only goes to the audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deny {
    McpOff,
    WorkspaceOff,
    DmsExcluded,
    NotAllowListed,
    DenyListed,
    WritesOff,
}

impl Deny {
    pub fn code(&self) -> &'static str {
        match self {
            Deny::McpOff => "mcp_off",
            Deny::WorkspaceOff => "workspace_off",
            Deny::DmsExcluded => "dms_excluded",
            Deny::NotAllowListed => "not_allow_listed",
            Deny::DenyListed => "deny_listed",
            Deny::WritesOff => "writes_off",
        }
    }
}

/// Whether a permitted write waits for the user's yes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    Confirm,
    Auto,
}

/// A policy over one settings snapshot; build a new one per call.
#[derive(Debug, Clone)]
pub struct Policy {
    settings: Arc<McpSettings>,
}

impl Policy {
    pub fn new(settings: Arc<McpSettings>) -> Self {
        Self { settings }
    }

    /// A policy over the source's current snapshot.
    pub fn from_source(source: &dyn SettingsSource) -> Self {
        Self::new(source.mcp())
    }

    /// The workspace's settings when MCP and the workspace are both on.
    pub fn workspace(&self, ws: &WorkspaceKey) -> Result<McpWorkspaceSettings, Deny> {
        if !self.settings.enabled {
            return Err(Deny::McpOff);
        }
        let w = self.settings.workspace(ws);
        if !w.enabled {
            return Err(Deny::WorkspaceOff);
        }
        Ok(w)
    }

    /// Order: workspace, DMs, deny list, then a non-empty allow list. Lists
    /// match conversation ids exactly; names never match.
    pub fn conversation(&self, ws: &WorkspaceKey, c: &Conversation) -> Result<(), Deny> {
        let w = self.workspace(ws)?;
        if matches!(c.kind, ConversationKind::Direct | ConversationKind::Group) && !w.include_dms {
            return Err(Deny::DmsExcluded);
        }
        let id: &str = &c.id.0;
        if w.deny_channels.iter().any(|d| d == id) {
            return Err(Deny::DenyListed);
        }
        if !w.allow_channels.is_empty() && !w.allow_channels.iter().any(|a| a == id) {
            return Err(Deny::NotAllowListed);
        }
        Ok(())
    }

    pub fn write(&self, ws: &WorkspaceKey) -> Result<WriteMode, Deny> {
        let w = self.workspace(ws)?;
        if !w.allow_writes {
            return Err(Deny::WritesOff);
        }
        Ok(if w.confirm_writes {
            WriteMode::Confirm
        } else {
            WriteMode::Auto
        })
    }

    /// The conversations a client may see, in input order.
    pub fn visible<'a>(
        &self,
        ws: &WorkspaceKey,
        cs: impl IntoIterator<Item = &'a Conversation>,
    ) -> Vec<&'a Conversation> {
        cs.into_iter()
            .filter(|c| self.conversation(ws, c).is_ok())
            .collect()
    }
}

/// How many "Always for this session" answers one session keeps. Past it
/// the prompt keeps asking instead of the set growing.
pub const MAX_SESSION_ALLOWS: usize = 64;

/// What one "Always for this session" answer covers: one write tool to one
/// destination (the conversation, or the workspace for `set_status`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionScope {
    pub tool: &'static str,
    pub workspace: WorkspaceKey,
    pub conversation: Option<ConversationId>,
}

/// The writes the user allowed for the rest of this session (one client
/// connection). Held in memory only, never written to settings or disk, so
/// a reconnect or an app restart asks again. The settings policy still runs
/// on every call; this only skips the prompt.
#[derive(Debug, Default)]
pub struct SessionAllows {
    scopes: Mutex<HashSet<SessionScope>>,
}

impl SessionAllows {
    /// Remembers `scope`. False, and nothing kept, when the set is full.
    pub fn allow(&self, scope: SessionScope) -> bool {
        let mut s = self.scopes.lock().unwrap_or_else(|e| e.into_inner());
        if s.contains(&scope) {
            return true;
        }
        if s.len() >= MAX_SESSION_ALLOWS {
            return false;
        }
        s.insert(scope)
    }

    pub fn allows(&self, scope: &SessionScope) -> bool {
        self.scopes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(scope)
    }

    pub fn len(&self) -> usize {
        self.scopes.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
