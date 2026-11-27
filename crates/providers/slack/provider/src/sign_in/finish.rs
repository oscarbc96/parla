use std::path::Path;

use components::sign_in::{Candidate, SignInError};
use settings::WorkspaceKey;
use slack_api::SessionCookie;
use slack_auth::{
    Account, AccountIndex, AuthError, CredentialStore, DiscoveredWorkspace, SessionStatus,
    WorkspaceCredentials, persist_verified, verify_discovered,
};

use crate::factory::CredentialBackend;

/// `CredentialStore` over the object-safe backend.
struct Backend<'a>(&'a dyn CredentialBackend);

impl CredentialStore for Backend<'_> {
    async fn save(&self, c: &WorkspaceCredentials) -> Result<(), AuthError> {
        self.0.save(c).await
    }
    async fn load(&self, ws: &WorkspaceKey) -> Result<Option<WorkspaceCredentials>, AuthError> {
        self.0.load(ws).await
    }
    async fn delete(&self, ws: &WorkspaceKey) -> Result<(), AuthError> {
        self.0.delete(ws).await
    }
}

fn io(_: std::io::Error) -> SignInError {
    SignInError::Failed(super::text("signin-error-accounts-update"))
}

/// Verifies every discovered workspace with `auth.test`, saves the valid ones
/// to the credential store and the account index, and returns one candidate
/// per workspace (`ready` when saved).
pub async fn complete_capture(
    backend: &dyn CredentialBackend,
    accounts_path: &Path,
    cookie: SessionCookie,
    found: Vec<DiscoveredWorkspace>,
) -> Result<Vec<Candidate>, SignInError> {
    let verified = verify_discovered(&cookie, found).await;
    let rows: Vec<(Account, String, bool)> = verified
        .iter()
        .map(|(w, s)| {
            let (user_id, ok) = match s {
                SessionStatus::Valid { user_id, .. } => (user_id.clone(), true),
                _ => (String::new(), false),
            };
            (
                Account {
                    workspace: w.workspace.clone(),
                    name: w.name.clone(),
                    url: w.url.clone(),
                    user_id,
                    enterprise_id: w.enterprise_id.clone(),
                    enterprise_name: None,
                    icon_url: None,
                },
                w.name.clone(),
                ok,
            )
        })
        .collect();
    let saved = persist_verified(&Backend(backend), &cookie, verified)
        .await
        .map_err(|e| SignInError::Failed(super::auth_error_text(&e)))?;
    let mut out = Vec::new();
    let mut ready_accounts = Vec::new();
    for (account, name, _) in rows {
        let ready = saved.contains(&account.workspace);
        out.push(Candidate {
            key: account.workspace.clone(),
            name,
            ready,
        });
        if ready {
            ready_accounts.push(account);
        }
    }
    AccountIndex::update(accounts_path, |index| {
        for a in ready_accounts {
            index.upsert(a);
        }
    })
    .map_err(io)?;
    Ok(out)
}

/// Deletes the keychain entry and the account row.
pub async fn sign_out(
    backend: &dyn CredentialBackend,
    accounts_path: &Path,
    ws: &WorkspaceKey,
) -> Result<(), String> {
    backend.delete(ws).await.map_err(|e| e.to_string())?;
    AccountIndex::update(accounts_path, |index| index.remove(ws))
        .map_err(|_| "The account list could not be updated.".to_string())
}
