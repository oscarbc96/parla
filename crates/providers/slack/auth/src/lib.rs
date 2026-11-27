//! Sign-in parsing for the Slack provider: local config, boot
//! data and manual paste. Nothing here logs or displays a secret.

mod accounts;
mod discover;
mod error;
mod manual;
pub mod native;
mod store;
mod verify;

pub use accounts::{Account, AccountIndex};
pub use store::{
    CredentialStore, KeychainStore, MemoryStore, WorkspaceCredentials, decode_credentials,
    encode_credentials, entry_url,
};
pub use verify::{SessionStatus, verify, verify_discovered};

pub use discover::{DiscoveredWorkspace, READ_TOKENS_SCRIPT, is_workspace_url, parse_local_config};
pub use error::AuthError;
pub mod webview;
pub use manual::{MANUAL_CONSOLE_SNIPPET, ManualInput, parse_manual};
pub use webview::persist_verified;
