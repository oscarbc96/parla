use std::path::PathBuf;
use std::sync::Arc;

use domain::{
    AccountInfo, ChatProvider, ContentParser, ExtCodec, ImageRef, OrgInfo, ParsedPermalink,
    ProviderError, ProviderFactory,
};
use futures::future::BoxFuture;
use settings::{ProviderId, WorkspaceKey};
use slack_api::{ClientConfig, WebClient};
use slack_auth::{
    AccountIndex, AuthError, CredentialStore, MemoryStore, SessionStatus, WorkspaceCredentials,
};
use url::Url;

use crate::provider::{SlackProvider, WsOptions, content_parser, ext_codec};

/// Where stored credentials come from (the OS keychain in the app).
pub trait CredentialSource: Send + Sync {
    fn load<'a>(
        &'a self,
        ws: &'a WorkspaceKey,
    ) -> BoxFuture<'a, Result<Option<WorkspaceCredentials>, AuthError>>;
}

impl CredentialSource for MemoryStore {
    fn load<'a>(
        &'a self,
        ws: &'a WorkspaceKey,
    ) -> BoxFuture<'a, Result<Option<WorkspaceCredentials>, AuthError>> {
        Box::pin(CredentialStore::load(self, ws))
    }
}

/// A credential source that can also save and delete (sign-in and sign-out).
pub trait CredentialBackend: CredentialSource {
    fn save<'a>(&'a self, c: &'a WorkspaceCredentials) -> BoxFuture<'a, Result<(), AuthError>>;
    fn delete<'a>(&'a self, ws: &'a WorkspaceKey) -> BoxFuture<'a, Result<(), AuthError>>;
}

impl CredentialBackend for MemoryStore {
    fn save<'a>(&'a self, c: &'a WorkspaceCredentials) -> BoxFuture<'a, Result<(), AuthError>> {
        Box::pin(CredentialStore::save(self, c))
    }
    fn delete<'a>(&'a self, ws: &'a WorkspaceKey) -> BoxFuture<'a, Result<(), AuthError>> {
        Box::pin(CredentialStore::delete(self, ws))
    }
}

pub struct SlackFactory {
    id: ProviderId,
    rt: tokio::runtime::Handle,
    accounts_path: PathBuf,
    creds: Arc<dyn CredentialSource>,
    backend: Option<Arc<dyn CredentialBackend>>,
    ws: WsOptions,
}

impl SlackFactory {
    pub fn new(
        rt: tokio::runtime::Handle,
        accounts_path: PathBuf,
        creds: Arc<dyn CredentialSource>,
    ) -> SlackFactory {
        SlackFactory {
            id: ProviderId("slack".to_string()),
            rt,
            accounts_path,
            creds,
            backend: None,
            ws: WsOptions::default(),
        }
    }

    /// The backend `remove` deletes workspace credentials from. Without it,
    /// `remove` is `Unsupported`.
    pub fn with_backend(mut self, backend: Arc<dyn CredentialBackend>) -> SlackFactory {
        self.backend = Some(backend);
        self
    }

    pub fn with_ws_options(mut self, ws: WsOptions) -> SlackFactory {
        self.ws = ws;
        self
    }
}

/// The registry entry for this provider (id `slack`). `accounts_path` is the
/// account index file; `creds` loads the stored credentials.
pub fn factory(
    rt: tokio::runtime::Handle,
    accounts_path: PathBuf,
    creds: Arc<dyn CredentialSource>,
) -> impl ProviderFactory {
    SlackFactory::new(rt, accounts_path, creds)
}

impl ProviderFactory for SlackFactory {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn display_name(&self) -> &str {
        "Slack"
    }

    fn accounts(&self) -> Vec<WorkspaceKey> {
        AccountIndex::load(&self.accounts_path)
            .map(|i| i.accounts().iter().map(|a| a.workspace.clone()).collect())
            .unwrap_or_default()
    }

    fn start(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Arc<dyn ChatProvider>, ProviderError>> {
        let ws = ws.clone();
        Box::pin(async move {
            let creds = self
                .creds
                .load(&ws)
                .await
                .map_err(|e| ProviderError::Other(e.to_string()))?
                // No stored credentials: the user has to sign in again.
                .ok_or(ProviderError::SessionExpired)?;
            let cfg = ClientConfig::for_workspace(&creds.url);
            match slack_auth::verify(&creds, cfg.clone()).await {
                SessionStatus::Valid { .. } => {}
                SessionStatus::Expired => return Err(ProviderError::SessionExpired),
                SessionStatus::Unreachable(m) => return Err(ProviderError::Network(m)),
            }
            let p = SlackProvider::start_with(creds, cfg, self.ws.clone(), &self.rt);
            // Off the start path: `team.info` never delays the workspace.
            p.spawn_account_refresh(self.accounts_path.clone(), &self.rt);
            Ok(Arc::new(p) as Arc<dyn ChatProvider>)
        })
    }

    fn account_info(&self) -> Vec<AccountInfo> {
        let Ok(index) = AccountIndex::load(&self.accounts_path) else {
            return Vec::new();
        };
        index
            .accounts()
            .iter()
            .map(|a| AccountInfo {
                key: a.workspace.clone(),
                name: Arc::from(a.name.as_str()),
                org: match (&a.enterprise_id, &a.enterprise_name) {
                    (Some(id), name) => Some(OrgInfo {
                        id: Arc::from(id.as_str()),
                        name: Arc::from(name.as_deref().unwrap_or(id.as_str())),
                    }),
                    _ => None,
                },
                icon: a.icon_url.as_deref().map(|u| ImageRef::Web(Arc::from(u))),
            })
            .collect()
    }

    fn remove(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<(), ProviderError>> {
        let ws = ws.clone();
        Box::pin(async move {
            let backend = self.backend.as_ref().ok_or(ProviderError::Unsupported)?;
            let other = |e: &dyn std::fmt::Display| ProviderError::Other(e.to_string());
            // Row first, credentials last: a failure part way leaves the
            // keychain item (the thing a retry needs), never an orphan secret.
            AccountIndex::update(&self.accounts_path, |i| i.remove(&ws)).map_err(|e| other(&e))?;
            // A missing item is already done, so a retry after a partial
            // failure completes. Token and cookie share one item per
            // workspace, so this touches no other workspace.
            if self.creds.load(&ws).await.map_err(|e| other(&e))?.is_some() {
                backend.delete(&ws).await.map_err(|e| other(&e))?;
            }
            Ok(())
        })
    }

    fn catalog(&self) -> &'static i18n::Catalog {
        crate::catalog()
    }

    fn ext_codec(&self) -> Arc<dyn ExtCodec> {
        ext_codec()
    }

    fn content_parser(&self) -> Arc<dyn ContentParser> {
        content_parser()
    }

    fn encoder(&self) -> Arc<dyn domain::ContentEncoder> {
        Arc::new(slack_markup::SlackEncoder)
    }

    fn parse_permalink(&self, url: &Url) -> Option<ParsedPermalink> {
        crate::permalink::parse_permalink(url)
    }
}

/// Re-reads `team.info` and stores the workspace name, icon and organisation
/// in the account index. One call per workspace start; no polling.
pub async fn refresh_account(
    accounts_path: &std::path::Path,
    ws: &WorkspaceKey,
    client: &WebClient,
) -> Result<(), ProviderError> {
    let info = client
        .team_info()
        .await
        .map_err(|e| ProviderError::Other(e.to_string()))?;
    let io = |e: std::io::Error| ProviderError::Other(e.to_string());
    // The load, check and save are one locked step, so a row removed meanwhile
    // is never put back.
    AccountIndex::update(accounts_path, |index| {
        let Some(mut account) = index
            .accounts()
            .iter()
            .find(|a| &a.workspace == ws)
            .cloned()
        else {
            return;
        };
        account.name = info.name;
        account.icon_url = info.icon;
        account.enterprise_id = info.enterprise_id;
        account.enterprise_name = info.enterprise_name;
        index.upsert(account);
    })
    .map_err(io)
}
