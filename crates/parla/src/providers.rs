//! The provider registry: the only file in `parla` that names a provider (the
//! boundary test exempts it). Also the keychain adapter that feeds stored
//! credentials to the provider factory.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use components::sign_in::SignIn;

use domain::ProviderFactory;
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use futures::future::BoxFuture;
use gpui::App;
use settings::WorkspaceKey;
use slack::{CredentialBackend, CredentialSource};
use slack_auth::{
    AuthError, CredentialStore, KeychainStore, WorkspaceCredentials, decode_credentials,
    encode_credentials, entry_url as credentials_url,
};
use zeroize::Zeroizing;

/// Every registered provider factory.
#[derive(Default, Clone)]
pub struct ProviderRegistry {
    factories: Vec<Arc<dyn ProviderFactory>>,
    sign_ins: Vec<Rc<dyn SignIn>>,
    /// Workspaces with a live session; shared by every clone of the registry.
    running: Arc<std::sync::Mutex<std::collections::HashSet<WorkspaceKey>>>,
}

/// Why `ProviderRegistry::remove_workspace_opt` did not finish. Every case can
/// be retried.
#[derive(Debug)]
pub enum RemoveError {
    /// The workspace still has a live session; stop it first.
    Running,
    Provider(domain::ProviderError),
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, f: impl ProviderFactory + 'static) {
        self.factories.push(Arc::new(f));
    }

    /// Every registered provider's catalog, in registration order.
    pub fn catalogs(&self) -> Vec<&'static i18n::Catalog> {
        self.factories.iter().map(|f| f.catalog()).collect()
    }

    /// Adds each provider's catalog to the app localizer (once each).
    pub fn register_catalogs(&self, cx: &mut gpui::App) {
        for catalog in self.catalogs() {
            i18n::gpui::add_catalog(cx, catalog);
        }
    }

    /// Registers a provider's sign-in screen.
    pub fn add_sign_in(&mut self, s: Rc<dyn SignIn>) {
        self.sign_ins.push(s);
    }

    /// The sign-in to show. With one provider it opens directly.
    pub fn sign_in(&self) -> Option<Rc<dyn SignIn>> {
        self.sign_ins.first().cloned()
    }

    /// Every registered sign-in, in registration order (the provider picker).
    pub fn sign_ins(&self) -> &[Rc<dyn SignIn>] {
        &self.sign_ins
    }

    /// Every registered provider factory.
    pub fn factories(&self) -> &[Arc<dyn ProviderFactory>] {
        &self.factories
    }

    /// The factory that owns `ws`.
    pub fn factory_for(&self, ws: &WorkspaceKey) -> Option<Arc<dyn ProviderFactory>> {
        self.factories
            .iter()
            .find(|f| f.id() == &ws.provider)
            .cloned()
    }

    /// Records that `ws` has (or no longer has) a live session. The shell
    /// calls this when it starts and when it drops a workspace session.
    pub fn set_running(&self, ws: &WorkspaceKey, running: bool) {
        let mut set = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if running {
            set.insert(ws.clone());
        } else {
            set.remove(ws);
        }
    }

    /// Signs `ws` out for good. Refused while its session is running (stop it
    /// first, so nothing writes to the store being deleted). Then the local
    /// store file, WAL and key go (when the app has one), then the provider
    /// drops the account row and credentials last. Each step is safe to repeat,
    /// so a failure part way is retried by calling this again. Other workspaces
    /// are untouched.
    pub async fn remove_workspace_opt(
        &self,
        ws: &WorkspaceKey,
        sqlite: Option<&store_sqlite::SqliteStore>,
    ) -> Result<(), RemoveError> {
        let factory = self
            .factory_for(ws)
            .ok_or(RemoveError::Provider(domain::ProviderError::Unsupported))?;
        if self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(ws)
        {
            return Err(RemoveError::Running);
        }
        if let Some(sqlite) = sqlite {
            sqlite
                .remove(ws)
                .await
                .map_err(|e| RemoveError::Provider(domain::ProviderError::Other(e.to_string())))?;
        }
        factory.remove(ws).await.map_err(RemoveError::Provider)
    }

    /// Every account's display row (name, organisation, icon), in registration order.
    pub fn account_infos(&self) -> Vec<domain::AccountInfo> {
        self.factories
            .iter()
            .flat_map(|f| f.account_info())
            .collect()
    }

    /// Every known account with the factory that owns it, in registration order.
    pub fn accounts(&self) -> Vec<(Arc<dyn ProviderFactory>, WorkspaceKey)> {
        self.factories
            .iter()
            .flat_map(|f| f.accounts().into_iter().map(|ws| (f.clone(), ws)))
            .collect()
    }
}

/// The app's registry: Slack with credentials from the OS keychain.
pub fn registry<C: CredentialBackend + 'static>(
    rt: tokio::runtime::Handle,
    creds: Arc<C>,
) -> ProviderRegistry {
    // The provider crates' timing lines (INFO) go to the log file.
    crate::log::add_forwarded_targets(&["slack_"]);
    #[cfg(feature = "fixture")]
    if let Some(all) = fixture_providers() {
        let mut r = ProviderRegistry::new();
        // The fixture's messages are written in the Slack provider's markup.
        let mut f =
            domain::test_support::FakeFactory::default().with_parser(Arc::new(slack::SlackParser));
        for p in all {
            f = f.with_provider(p);
        }
        r.add(f);
        return r;
    }
    let accounts_path: PathBuf = settings::paths::data_dir().join("accounts.json");
    let mut r = ProviderRegistry::new();
    r.add(
        slack::SlackFactory::new(rt.clone(), accounts_path.clone(), creds.clone())
            .with_backend(creds.clone()),
    );
    r.add_sign_in(Rc::new(slack::SlackSignIn::new(rt, accounts_path, creds)));
    r
}

/// The synthetic `perf` workspace when `PARLA_FIXTURE=perf` is
/// set (`perf3` registers three); shared instances, so the perf driver can push
/// events into them.
#[cfg(feature = "fixture")]
pub fn fixture_providers() -> Option<Vec<Arc<domain::test_support::FakeProvider>>> {
    static FIXTURE: std::sync::OnceLock<Vec<Arc<domain::test_support::FakeProvider>>> =
        std::sync::OnceLock::new();
    let kind = std::env::var("PARLA_FIXTURE").ok()?;
    matches!(
        kind.as_str(),
        "perf" | "perf3" | "rail-scale" | "sidebar-scale" | "names"
    )
    .then(|| {
        FIXTURE
            .get_or_init(|| match kind.as_str() {
                "sidebar-scale" => {
                    vec![Arc::new(
                        domain::test_support::fixture::sidebar_scale_workspace(),
                    )]
                }
                "names" => {
                    vec![Arc::new(
                        domain::test_support::fixture::unresolved_names_workspace(),
                    )]
                }
                // Nine workspaces: more than a short window's rail fits.
                "rail-scale" => domain::test_support::fixture::perf_workspaces(1, 9)
                    .into_iter()
                    .map(Arc::new)
                    .collect(),
                "perf3" => domain::test_support::fixture::perf_workspaces(1, 3)
                    .into_iter()
                    .map(Arc::new)
                    .collect(),
                _ => vec![Arc::new(domain::test_support::fixture::perf_workspace(1))],
            })
            .clone()
    })
}

/// The first fixture workspace, the one shown at start.
#[cfg(feature = "fixture")]
pub fn fixture_provider() -> Option<Arc<domain::test_support::FakeProvider>> {
    fixture_providers().and_then(|v| v.first().cloned())
}

type Reply<T> = oneshot::Sender<Result<T, AuthError>>;

enum Request {
    Load(WorkspaceKey, Reply<Option<WorkspaceCredentials>>),
    /// Credentials travel in their encoded, zeroizing form.
    Save(WorkspaceKey, String, Zeroizing<Vec<u8>>, Reply<()>),
    Delete(WorkspaceKey, Reply<()>),
}

fn unavailable() -> AuthError {
    AuthError::Keychain("keychain unavailable".to_string())
}

/// `CredentialSource`/`CredentialBackend` over [`KeychainStore`]. The keychain
/// needs GPUI's `AsyncApp`, which is not `Send`, so requests hop to the main
/// thread over a channel and a foreground task answers them.
pub struct KeychainCredentials {
    tx: mpsc::UnboundedSender<Request>,
}

impl KeychainCredentials {
    pub fn new(cx: &mut App) -> Self {
        let (tx, mut rx) = mpsc::unbounded::<Request>();
        cx.spawn(async move |cx| {
            let store = KeychainStore::new(cx.clone());
            while let Some(req) = rx.next().await {
                match req {
                    Request::Load(ws, reply) => {
                        let _ = reply.send(CredentialStore::load(&store, &ws).await);
                    }
                    Request::Save(ws, user, bytes, reply) => {
                        let r = match decode_credentials(&ws, &user, &bytes) {
                            Ok(c) => CredentialStore::save(&store, &c).await,
                            Err(e) => Err(e),
                        };
                        let _ = reply.send(r);
                    }
                    Request::Delete(ws, reply) => {
                        let _ = reply.send(CredentialStore::delete(&store, &ws).await);
                    }
                }
            }
        })
        .detach();
        Self { tx }
    }

    async fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Request) -> Result<T, AuthError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .unbounded_send(make(reply))
            .map_err(|_| unavailable())?;
        rx.await.map_err(|_| unavailable())?
    }
}

impl CredentialSource for KeychainCredentials {
    fn load<'a>(
        &'a self,
        ws: &'a WorkspaceKey,
    ) -> BoxFuture<'a, Result<Option<WorkspaceCredentials>, AuthError>> {
        Box::pin(self.ask(|r| Request::Load(ws.clone(), r)))
    }
}

impl CredentialBackend for KeychainCredentials {
    fn save<'a>(&'a self, c: &'a WorkspaceCredentials) -> BoxFuture<'a, Result<(), AuthError>> {
        let bytes = encode_credentials(c);
        Box::pin(self.ask(move |r| Request::Save(c.workspace.clone(), c.user_id.clone(), bytes, r)))
    }

    fn delete<'a>(&'a self, ws: &'a WorkspaceKey) -> BoxFuture<'a, Result<(), AuthError>> {
        Box::pin(self.ask(|r| Request::Delete(ws.clone(), r)))
    }
}

/// The credential store the app runs with: the keychain, or the debug-build
/// file (see [`crate::dev_credentials`]).
pub enum AppCredentials {
    Keychain(KeychainCredentials),
    Dev(DevCredentials),
}

impl CredentialSource for AppCredentials {
    fn load<'a>(
        &'a self,
        ws: &'a WorkspaceKey,
    ) -> BoxFuture<'a, Result<Option<WorkspaceCredentials>, AuthError>> {
        match self {
            Self::Keychain(c) => CredentialSource::load(c, ws),
            Self::Dev(c) => CredentialSource::load(c, ws),
        }
    }
}

impl CredentialBackend for AppCredentials {
    fn save<'a>(&'a self, c: &'a WorkspaceCredentials) -> BoxFuture<'a, Result<(), AuthError>> {
        match self {
            Self::Keychain(k) => CredentialBackend::save(k, c),
            Self::Dev(k) => CredentialBackend::save(k, c),
        }
    }

    fn delete<'a>(&'a self, ws: &'a WorkspaceKey) -> BoxFuture<'a, Result<(), AuthError>> {
        match self {
            Self::Keychain(k) => CredentialBackend::delete(k, ws),
            Self::Dev(k) => CredentialBackend::delete(k, ws),
        }
    }
}

fn auth_err(_: std::io::Error) -> AuthError {
    AuthError::Keychain("development credentials file unavailable".to_string())
}

/// Credentials in the dev file.
#[derive(Clone)]
pub struct DevCredentials(pub Arc<crate::dev_credentials::DevFile>);

impl CredentialSource for DevCredentials {
    fn load<'a>(
        &'a self,
        ws: &'a WorkspaceKey,
    ) -> BoxFuture<'a, Result<Option<WorkspaceCredentials>, AuthError>> {
        Box::pin(async move {
            match self.0.get(&credentials_url(ws)).map_err(auth_err)? {
                Some((user, bytes)) => decode_credentials(ws, &user, &bytes).map(Some),
                None => Ok(None),
            }
        })
    }
}

impl CredentialBackend for DevCredentials {
    fn save<'a>(&'a self, c: &'a WorkspaceCredentials) -> BoxFuture<'a, Result<(), AuthError>> {
        Box::pin(async move {
            let bytes = encode_credentials(c);
            self.0
                .put(&credentials_url(&c.workspace), &c.user_id, &bytes)
                .map_err(auth_err)
        })
    }

    fn delete<'a>(&'a self, ws: &'a WorkspaceKey) -> BoxFuture<'a, Result<(), AuthError>> {
        Box::pin(async move { self.0.remove(&credentials_url(ws)).map_err(auth_err) })
    }
}

type Cached = (String, Zeroizing<Vec<u8>>);

/// Reads each workspace's credentials from the inner backend at most once per
/// process: the OS keychain prompts on reads, so reconnects use memory. The
/// cache holds the encoded form, zeroized on drop; `save` and `delete` keep it
/// in step.
pub struct CachingCredentials<C> {
    inner: C,
    cache: futures::lock::Mutex<std::collections::HashMap<WorkspaceKey, Cached>>,
}

impl<C: CredentialBackend> CachingCredentials<C> {
    pub fn new(inner: C) -> Arc<Self> {
        Arc::new(Self::with(inner))
    }

    fn with(inner: C) -> Self {
        Self {
            inner,
            cache: Default::default(),
        }
    }
}

impl<C: CredentialBackend> CredentialSource for CachingCredentials<C> {
    fn load<'a>(
        &'a self,
        ws: &'a WorkspaceKey,
    ) -> BoxFuture<'a, Result<Option<WorkspaceCredentials>, AuthError>> {
        Box::pin(async move {
            let mut cache = self.cache.lock().await;
            if let Some((user, bytes)) = cache.get(ws) {
                return decode_credentials(ws, user, bytes).map(Some);
            }
            let loaded = self.inner.load(ws).await?;
            if let Some(c) = &loaded {
                cache.insert(ws.clone(), (c.user_id.clone(), encode_credentials(c)));
            }
            Ok(loaded)
        })
    }
}

impl<C: CredentialBackend> CredentialBackend for CachingCredentials<C> {
    fn save<'a>(&'a self, c: &'a WorkspaceCredentials) -> BoxFuture<'a, Result<(), AuthError>> {
        Box::pin(async move {
            let mut cache = self.cache.lock().await;
            cache.remove(&c.workspace);
            self.inner.save(c).await?;
            cache.insert(
                c.workspace.clone(),
                (c.user_id.clone(), encode_credentials(c)),
            );
            Ok(())
        })
    }

    fn delete<'a>(&'a self, ws: &'a WorkspaceKey) -> BoxFuture<'a, Result<(), AuthError>> {
        Box::pin(async move {
            let mut cache = self.cache.lock().await;
            cache.remove(ws);
            self.inner.delete(ws).await
        })
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use slack_auth::MemoryStore;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting {
        loads: Arc<AtomicUsize>,
        mem: MemoryStore,
    }
    impl CredentialSource for Counting {
        fn load<'a>(
            &'a self,
            ws: &'a WorkspaceKey,
        ) -> BoxFuture<'a, Result<Option<WorkspaceCredentials>, AuthError>> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            CredentialSource::load(&self.mem, ws)
        }
    }
    impl CredentialBackend for Counting {
        fn save<'a>(&'a self, c: &'a WorkspaceCredentials) -> BoxFuture<'a, Result<(), AuthError>> {
            CredentialBackend::save(&self.mem, c)
        }
        fn delete<'a>(&'a self, ws: &'a WorkspaceKey) -> BoxFuture<'a, Result<(), AuthError>> {
            CredentialBackend::delete(&self.mem, ws)
        }
    }

    fn creds(ws: &WorkspaceKey, token: &str) -> WorkspaceCredentials {
        let json = format!(
            r#"{{"v":1,"token":"{token}","cookie":"xoxd-abc","url":"https://example.slack.com/"}}"#
        );
        decode_credentials(ws, "U1", json.as_bytes()).unwrap()
    }

    fn setup() -> (CachingCredentials<Counting>, Arc<AtomicUsize>, WorkspaceKey) {
        let loads = Arc::new(AtomicUsize::new(0));
        let c = CachingCredentials::with(Counting {
            loads: loads.clone(),
            mem: MemoryStore::default(),
        });
        (c, loads, WorkspaceKey::new("slack", "T1"))
    }

    #[test]
    fn one_inner_load_across_reconnects() {
        let (c, loads, ws) = setup();
        futures::executor::block_on(async {
            // Seed the inner store directly so the first load must hit it.
            CredentialBackend::save(&c.inner, &creds(&ws, "xoxc-1"))
                .await
                .unwrap();
            for _ in 0..4 {
                let got = CredentialSource::load(&c, &ws).await.unwrap().unwrap();
                assert_eq!(got.token.expose(), "xoxc-1");
            }
        });
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn save_and_delete_keep_cache_in_step() {
        let (c, loads, ws) = setup();
        futures::executor::block_on(async {
            CredentialBackend::save(&c, &creds(&ws, "xoxc-2"))
                .await
                .unwrap();
            let got = CredentialSource::load(&c, &ws).await.unwrap().unwrap();
            assert_eq!(got.token.expose(), "xoxc-2");
            assert_eq!(loads.load(Ordering::SeqCst), 0);
            CredentialBackend::delete(&c, &ws).await.unwrap();
            assert!(CredentialSource::load(&c, &ws).await.unwrap().is_none());
            assert_eq!(loads.load(Ordering::SeqCst), 1);
        });
    }
}

#[cfg(test)]
mod dev_tests {
    use super::*;
    use crate::dev_credentials::{DevFile, FILE_NAME};

    fn ws() -> WorkspaceKey {
        WorkspaceKey::new("slack", "T1")
    }

    fn creds() -> WorkspaceCredentials {
        let json =
            r#"{"v":1,"token":"xoxc-abc","cookie":"xoxd-abc","url":"https://example.slack.com/"}"#;
        decode_credentials(&ws(), "U1", json.as_bytes()).unwrap()
    }

    #[test]
    fn credentials_round_trip_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let c = DevCredentials(DevFile::new(dir.path().join("d").join(FILE_NAME)));
        futures::executor::block_on(async {
            assert!(CredentialSource::load(&c, &ws()).await.unwrap().is_none());
            CredentialBackend::save(&c, &creds()).await.unwrap();
            let got = CredentialSource::load(&c, &ws()).await.unwrap().unwrap();
            assert_eq!(got.token.expose(), "xoxc-abc");
            assert_eq!(got.user_id, "U1");
            CredentialBackend::delete(&c, &ws()).await.unwrap();
            assert!(CredentialSource::load(&c, &ws()).await.unwrap().is_none());
        });
    }

    #[test]
    fn credentials_and_keys_share_one_file() {
        use store_sqlite::KeySource as _;
        let dir = tempfile::tempdir().unwrap();
        let f = DevFile::new(dir.path().join("d").join(FILE_NAME));
        let (c, k) = (
            DevCredentials(f.clone()),
            crate::dev_credentials::DevKeys(f),
        );
        futures::executor::block_on(async {
            CredentialBackend::save(&c, &creds()).await.unwrap();
            let key = k
                .get_or_create(&store_sqlite::DbId::Workspace(ws()))
                .await
                .unwrap();
            assert!(CredentialSource::load(&c, &ws()).await.unwrap().is_some());
            assert_eq!(
                *k.get_or_create(&store_sqlite::DbId::Workspace(ws()))
                    .await
                    .unwrap()
                    .to_hex(),
                *key.to_hex()
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn credentials_file_is_0600() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let f = DevFile::new(dir.path().join("d").join(FILE_NAME));
        let c = DevCredentials(f.clone());
        futures::executor::block_on(CredentialBackend::save(&c, &creds())).unwrap();
        let mode = std::fs::metadata(f.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

/// Redaction rules of every registered provider, for reports written outside
/// the log path (crash reports).
pub fn redaction_rules() -> Arc<dyn redact::RuleSet> {
    Arc::new(slack_redact::Rules::default())
}
