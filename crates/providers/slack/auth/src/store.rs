//! Credential storage. The trait keeps tests off the real OS keychain.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use settings::WorkspaceKey;
use slack_api::{SessionCookie, XoxcToken};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use crate::AuthError;

/// Everything needed to talk to one workspace. `Debug` redacts both secrets.
#[derive(Debug)]
pub struct WorkspaceCredentials {
    pub workspace: WorkspaceKey,
    pub user_id: String,
    pub url: Url,
    pub token: XoxcToken,
    pub cookie: SessionCookie,
}

pub trait CredentialStore {
    fn save(&self, c: &WorkspaceCredentials) -> impl Future<Output = Result<(), AuthError>>;
    fn load(
        &self,
        ws: &WorkspaceKey,
    ) -> impl Future<Output = Result<Option<WorkspaceCredentials>, AuthError>>;
    fn delete(&self, ws: &WorkspaceKey) -> impl Future<Output = Result<(), AuthError>>;
}

/// Keychain entry URL for a workspace: one item per team.
pub fn entry_url(ws: &WorkspaceKey) -> String {
    format!("parla://workspace/{}", ws.id.0)
}

#[derive(Serialize)]
struct WireOut<'a> {
    v: u32,
    token: &'a str,
    cookie: &'a str,
    url: &'a str,
}

#[derive(Deserialize)]
struct WireIn {
    v: u32,
    token: String,
    cookie: String,
    url: String,
}

/// Serializes the secret payload into a buffer that is zeroized on drop.
pub fn encode_credentials(c: &WorkspaceCredentials) -> Zeroizing<Vec<u8>> {
    let cookie = Zeroizing::new(c.cookie.header_value());
    let bytes = serde_json::to_vec(&WireOut {
        v: 1,
        token: c.token.expose(),
        cookie: &cookie,
        url: c.url.as_str(),
    })
    .expect("plain strings always serialize");
    Zeroizing::new(bytes)
}

/// Inverse of [`encode_credentials`]. Errors never include the payload.
pub fn decode_credentials(
    ws: &WorkspaceKey,
    user_id: &str,
    bytes: &[u8],
) -> Result<WorkspaceCredentials, AuthError> {
    let mut w: WireIn = serde_json::from_slice(bytes)
        .map_err(|_| AuthError::Keychain("stored credentials are unreadable".to_string()))?;
    let out = (|| {
        if w.v != 1 {
            return Err(AuthError::Keychain(
                "unsupported stored credentials version".to_string(),
            ));
        }
        Ok(WorkspaceCredentials {
            workspace: ws.clone(),
            user_id: user_id.to_string(),
            url: Url::parse(&w.url)
                .ok()
                .filter(crate::is_workspace_url)
                .ok_or_else(|| {
                    AuthError::Keychain("stored workspace url is invalid".to_string())
                })?,
            token: XoxcToken::parse(&w.token)?,
            cookie: SessionCookie::parse(&w.cookie)?,
        })
    })();
    w.token.zeroize();
    w.cookie.zeroize();
    out
}

/// One stored entry: the account label and the encoded secret payload.
type StoredItem = (String, Zeroizing<Vec<u8>>);

/// In-memory store for tests and previews. Holds the same encoded payload the
/// keychain would.
#[derive(Default)]
pub struct MemoryStore {
    items: Mutex<HashMap<WorkspaceKey, StoredItem>>,
}

impl CredentialStore for MemoryStore {
    async fn save(&self, c: &WorkspaceCredentials) -> Result<(), AuthError> {
        self.items.lock().unwrap().insert(
            c.workspace.clone(),
            (c.user_id.clone(), encode_credentials(c)),
        );
        Ok(())
    }

    async fn load(&self, ws: &WorkspaceKey) -> Result<Option<WorkspaceCredentials>, AuthError> {
        let items = self.items.lock().unwrap();
        items
            .get(ws)
            .map(|(user, bytes)| decode_credentials(ws, user, bytes))
            .transpose()
    }

    async fn delete(&self, ws: &WorkspaceKey) -> Result<(), AuthError> {
        self.items.lock().unwrap().remove(ws);
        Ok(())
    }
}

/// The OS keychain through GPUI's credentials API. GPUI's test platform stubs
/// these calls, so this is covered by an ignored smoke test only.
pub struct KeychainStore {
    cx: gpui::AsyncApp,
}

impl KeychainStore {
    pub fn new(cx: gpui::AsyncApp) -> KeychainStore {
        KeychainStore { cx }
    }
}

fn keychain_err(e: impl std::fmt::Display) -> AuthError {
    AuthError::Keychain(e.to_string())
}

impl CredentialStore for KeychainStore {
    async fn save(&self, c: &WorkspaceCredentials) -> Result<(), AuthError> {
        let url = entry_url(&c.workspace);
        let payload = encode_credentials(c);
        let task = self
            .cx
            .update(|cx| cx.write_credentials(&url, &c.user_id, &payload));
        task.await.map_err(keychain_err)
    }

    async fn load(&self, ws: &WorkspaceKey) -> Result<Option<WorkspaceCredentials>, AuthError> {
        let url = entry_url(ws);
        let found = self
            .cx
            .update(|cx| cx.read_credentials(&url))
            .await
            .map_err(keychain_err)?;
        let Some((user, bytes)) = found else {
            return Ok(None);
        };
        let bytes = Zeroizing::new(bytes);
        decode_credentials(ws, &user, &bytes).map(Some)
    }

    async fn delete(&self, ws: &WorkspaceKey) -> Result<(), AuthError> {
        let url = entry_url(ws);
        self.cx
            .update(|cx| cx.delete_credentials(&url))
            .await
            .map_err(keychain_err)
    }
}
