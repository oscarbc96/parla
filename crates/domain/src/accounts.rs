//! What the shell knows about each signed-in account.

use std::sync::Arc;

use settings::WorkspaceKey;

use crate::rich::ImageRef;

/// The organisation an account belongs to (Enterprise Grid style nesting).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrgInfo {
    pub id: Arc<str>,
    pub name: Arc<str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccountInfo {
    pub key: WorkspaceKey,
    pub name: Arc<str>,
    pub org: Option<OrgInfo>,
    pub icon: Option<ImageRef>,
}
