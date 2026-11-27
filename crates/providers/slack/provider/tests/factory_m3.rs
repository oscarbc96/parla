use std::path::Path;
use std::sync::Arc;

use domain::{ImageRef, ProviderError, ProviderFactory};
use settings::WorkspaceKey;
use slack::{SlackFactory, refresh_account};
use slack_api::{ClientConfig, SessionCookie, WebClient, XoxcToken};
use slack_auth::{Account, AccountIndex, CredentialStore, MemoryStore, WorkspaceCredentials};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn key(id: &str) -> WorkspaceKey {
    WorkspaceKey::new("slack", id)
}

fn account(id: &str, org: Option<(&str, &str)>, icon: Option<&str>) -> Account {
    Account {
        workspace: key(id),
        name: format!("Team {id}"),
        url: Url::parse("https://example.slack.com/").unwrap(),
        user_id: "U0000001".into(),
        enterprise_id: org.map(|o| o.0.to_string()),
        enterprise_name: org.map(|o| o.1.to_string()),
        icon_url: icon.map(str::to_string),
    }
}

fn creds(id: &str) -> WorkspaceCredentials {
    WorkspaceCredentials {
        workspace: key(id),
        user_id: "U0000001".into(),
        url: Url::parse("https://example.slack.com/").unwrap(),
        token: XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        cookie: SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    }
}

async fn setup(dir: &Path, accounts: Vec<Account>) -> (SlackFactory, Arc<MemoryStore>) {
    let path = dir.join("accounts.json");
    let mut idx = AccountIndex::load(&path).unwrap();
    let store = Arc::new(MemoryStore::default());
    for a in accounts {
        store.save(&creds(&a.workspace.id.0)).await.unwrap();
        idx.upsert(a);
    }
    idx.save().unwrap();
    let f = SlackFactory::new(tokio::runtime::Handle::current(), path, store.clone())
        .with_backend(store.clone());
    (f, store)
}

#[tokio::test]
async fn account_info_groups_enterprise() {
    let dir = tempfile::tempdir().unwrap();
    let (f, _) = setup(
        dir.path(),
        vec![
            account("T0000002", Some(("E0000001", "Example Org")), None),
            account("T0000001", None, Some("https://example.com/i.png")),
            account("T0000003", Some(("E0000001", "Example Org")), None),
        ],
    )
    .await;
    let info = f.account_info();
    let ids: Vec<_> = info.iter().map(|i| i.key.id.0.as_str()).collect();
    assert_eq!(ids, ["T0000002", "T0000001", "T0000003"], "index order");
    assert_eq!(info[0].org, info[2].org);
    let org = info[0].org.as_ref().unwrap();
    assert_eq!((&*org.id, &*org.name), ("E0000001", "Example Org"));
    assert_eq!(info[0].name.as_ref(), "Team T0000002");
    assert!(info[1].org.is_none());
    assert_eq!(
        info[1].icon,
        Some(ImageRef::Web("https://example.com/i.png".into()))
    );
}

#[tokio::test]
async fn remove_deletes_token_and_row() {
    let dir = tempfile::tempdir().unwrap();
    let (f, store) = setup(
        dir.path(),
        vec![
            account("T0000001", None, None),
            account("T0000002", None, None),
        ],
    )
    .await;
    f.remove(&key("T0000001")).await.unwrap();
    assert!(
        CredentialStore::load(&*store, &key("T0000001"))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(f.accounts(), vec![key("T0000002")]);
    let idx = AccountIndex::load(&dir.path().join("accounts.json")).unwrap();
    assert_eq!(idx.accounts().len(), 1);
    // Removing it again is fine.
    f.remove(&key("T0000001")).await.unwrap();
}

#[tokio::test]
async fn cookie_deleted_with_last_workspace_only() {
    // Each workspace's keychain entry carries its own copy of the session
    // cookie, so removing one workspace must leave the others' cookies usable
    // and removing the last one leaves no credential behind.
    let dir = tempfile::tempdir().unwrap();
    let (f, store) = setup(
        dir.path(),
        vec![
            account("T0000001", None, None),
            account("T0000002", None, None),
        ],
    )
    .await;
    f.remove(&key("T0000001")).await.unwrap();
    let kept = CredentialStore::load(&*store, &key("T0000002"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.cookie.header_value(), "d=xoxd-secretcookie%2Fvalue");
    f.remove(&key("T0000002")).await.unwrap();
    assert!(
        CredentialStore::load(&*store, &key("T0000002"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(f.accounts().is_empty());
}

#[tokio::test]
async fn remove_without_backend_is_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.json");
    let f = SlackFactory::new(
        tokio::runtime::Handle::current(),
        path,
        Arc::new(MemoryStore::default()),
    );
    assert!(matches!(
        f.remove(&key("T0000001")).await,
        Err(ProviderError::Unsupported)
    ));
}

#[tokio::test]
async fn refresh_updates_icon_and_org() {
    let server = MockServer::start().await;
    let body = std::fs::read_to_string(format!(
        "{}/../api/tests/fixtures/team/info_enterprise.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
    .replace(
        "\"image_default\":true",
        "\"image_default\":false,\"image_132\":\"https://example.com/i132.png\"",
    );
    Mock::given(method("POST"))
        .and(path("/api/team.info"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("accounts.json");
    let mut idx = AccountIndex::load(&p).unwrap();
    idx.upsert(account("T0000002", None, None));
    idx.save().unwrap();
    let client = WebClient::new(
        ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap()),
        XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    )
    .unwrap();
    refresh_account(&p, &key("T0000002"), &client)
        .await
        .unwrap();
    let a = AccountIndex::load(&p).unwrap().accounts()[0].clone();
    assert_eq!(a.enterprise_id.as_deref(), Some("E0000001"));
    assert_eq!(a.enterprise_name.as_deref(), Some("Example Org"));
    assert_eq!(a.icon_url.as_deref(), Some("https://example.com/i132.png"));
    assert_eq!(a.name, "Example Workspace");
}

/// Errors when asked to delete an item that is not there (like a keychain),
/// and can be told to fail the next delete.
#[derive(Default)]
struct Flaky {
    inner: MemoryStore,
    fail_next_delete: std::sync::atomic::AtomicBool,
}

impl slack::CredentialSource for Flaky {
    fn load<'a>(
        &'a self,
        ws: &'a WorkspaceKey,
    ) -> futures::future::BoxFuture<'a, Result<Option<WorkspaceCredentials>, slack_auth::AuthError>>
    {
        Box::pin(CredentialStore::load(&self.inner, ws))
    }
}

impl slack::CredentialBackend for Flaky {
    fn save<'a>(
        &'a self,
        c: &'a WorkspaceCredentials,
    ) -> futures::future::BoxFuture<'a, Result<(), slack_auth::AuthError>> {
        Box::pin(CredentialStore::save(&self.inner, c))
    }
    fn delete<'a>(
        &'a self,
        ws: &'a WorkspaceKey,
    ) -> futures::future::BoxFuture<'a, Result<(), slack_auth::AuthError>> {
        Box::pin(async move {
            use std::sync::atomic::Ordering;
            if self.fail_next_delete.swap(false, Ordering::SeqCst) {
                return Err(slack_auth::AuthError::Keychain("locked".into()));
            }
            if CredentialStore::load(&self.inner, ws).await?.is_none() {
                return Err(slack_auth::AuthError::Keychain("not found".into()));
            }
            CredentialStore::delete(&self.inner, ws).await
        })
    }
}

#[tokio::test]
async fn remove_is_retryable_after_a_keychain_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.json");
    let mut idx = AccountIndex::load(&path).unwrap();
    idx.upsert(account("T0000001", None, None));
    idx.upsert(account("T0000002", None, None));
    idx.save().unwrap();
    let flaky = Arc::new(Flaky::default());
    for id in ["T0000001", "T0000002"] {
        CredentialStore::save(&flaky.inner, &creds(id))
            .await
            .unwrap();
    }
    let f = SlackFactory::new(tokio::runtime::Handle::current(), path, flaky.clone())
        .with_backend(flaky.clone());

    flaky
        .fail_next_delete
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(f.remove(&key("T0000001")).await.is_err());
    // The row is gone and the secret is still there for the retry.
    assert_eq!(f.accounts(), vec![key("T0000002")]);
    assert!(
        CredentialStore::load(&flaky.inner, &key("T0000001"))
            .await
            .unwrap()
            .is_some()
    );

    f.remove(&key("T0000001")).await.unwrap();
    assert!(
        CredentialStore::load(&flaky.inner, &key("T0000001"))
            .await
            .unwrap()
            .is_none()
    );
    // Already gone: the missing item is success, not an error.
    f.remove(&key("T0000001")).await.unwrap();
    assert!(
        CredentialStore::load(&flaky.inner, &key("T0000002"))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn refresh_does_not_resurrect_a_removed_row_and_clears_stale_org() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/team.info"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"ok":true,"team":{"id":"T0000001","name":"Solo","icon":{"image_default":true}}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;
    let client = WebClient::new(
        ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap()),
        XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("accounts.json");
    AccountIndex::update(&p, |i| {
        i.upsert(account("T0000001", Some(("E0000001", "Old Org")), None))
    })
    .unwrap();
    refresh_account(&p, &key("T0000001"), &client)
        .await
        .unwrap();
    let a = AccountIndex::load(&p).unwrap().accounts()[0].clone();
    assert_eq!((a.enterprise_id, a.enterprise_name), (None, None));

    AccountIndex::update(&p, |i| i.remove(&key("T0000001"))).unwrap();
    refresh_account(&p, &key("T0000001"), &client)
        .await
        .unwrap();
    assert!(AccountIndex::load(&p).unwrap().accounts().is_empty());
}

#[tokio::test]
async fn factory_encoder_escapes_and_tokenises() {
    let dir = tempfile::tempdir().unwrap();
    let (f, _) = setup(dir.path(), vec![]).await;
    let t = domain::ComposedText::new(
        "@Ana <b>",
        vec![domain::ComposedSpan {
            range: 0..4,
            inline: domain::Inline::User {
                id: domain::UserId("U1".into()),
                label: None,
            },
        }],
    );
    assert_eq!(f.encoder().encode(&t), "<@U1> &lt;b&gt;");
}
