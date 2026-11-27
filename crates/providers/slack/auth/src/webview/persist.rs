use settings::WorkspaceKey;
use slack_api::SessionCookie;

use crate::{AuthError, CredentialStore, DiscoveredWorkspace, SessionStatus, WorkspaceCredentials};

/// Saves every workspace whose `auth.test` was valid (from `verify_discovered`) and
/// returns their keys. Expired or unreachable ones are skipped.
pub async fn persist_verified<S: CredentialStore>(
    store: &S,
    cookie: &SessionCookie,
    verified: Vec<(DiscoveredWorkspace, SessionStatus)>,
) -> Result<Vec<WorkspaceKey>, AuthError> {
    let mut saved = Vec::new();
    for (ws, status) in verified {
        let SessionStatus::Valid { user_id, .. } = status else {
            continue;
        };
        let cookie = cookie.duplicate();
        let creds = WorkspaceCredentials {
            workspace: ws.workspace.clone(),
            user_id,
            url: ws.url,
            token: ws.token,
            cookie,
        };
        store.save(&creds).await?;
        saved.push(ws.workspace);
    }
    Ok(saved)
}
