//! Provider sign-in pipeline: paste, capture completion, picker candidates and
//! sign out, against a mock `auth.test` and an in-memory credential store.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use components::sign_in::{SignIn as _, SignInError};
use components::workspace_picker::{WorkspaceChoice, WorkspacePickerModel};
use futures::channel::mpsc::UnboundedSender;
use futures::future::LocalBoxFuture;
use settings::WorkspaceKey;
use slack::CredentialBackend;
use slack::sign_in::{
    SlackSignIn, capture_failure, complete_capture, complete_manual, early_session, sign_out,
};
use slack_api::{SessionCookie, SigninWorkspace, XoxcToken};
use slack_auth::webview::{CaptureFailure, RawCookie, WebviewDriver};
use slack_auth::{
    Account, AccountIndex, AuthError, CredentialStore, MemoryStore, WorkspaceCredentials,
};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OK: &str = r#"{"ok":true,"url":"https://example.invalid/","team":"Example Team","user":"alice","team_id":"T0000001","user_id":"U0000001","is_enterprise_install":false}"#;
const BAD: &str = r#"{"ok":false,"error":"invalid_auth"}"#;
const TOKEN: &str = "xoxc-1-2-3-aaaa";
const COOKIE: &str = "xoxd-synthetic";

fn config(url: &str) -> String {
    format!(r#"{{"teams":{{"T0000001":{{"name":"Example","url":"{url}/","token":"{TOKEN}"}}}}}}"#)
}

async fn server(body: &'static str) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&s)
        .await;
    s
}

#[tokio::test]
async fn manual_paste_verifies_and_saves() {
    let s = server(OK).await;
    let dir = tempfile::tempdir().unwrap();
    let accounts = dir.path().join("accounts.json");
    let store = MemoryStore::default();
    let found = complete_manual(&store, &accounts, COOKIE, &config(&s.uri()))
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].ready);
    let ws = WorkspaceKey::new("slack", "T0000001");
    assert!(CredentialStore::load(&store, &ws).await.unwrap().is_some());
    let idx = AccountIndex::load(&accounts).unwrap();
    assert_eq!(idx.accounts().len(), 1);
    assert_eq!(idx.accounts()[0].user_id, "U0000001");
}

#[tokio::test]
async fn manual_paste_bad_token_shows_error_without_value() {
    let s = server(BAD).await;
    let dir = tempfile::tempdir().unwrap();
    let accounts = dir.path().join("accounts.json");
    let store = MemoryStore::default();
    let found = complete_manual(&store, &accounts, COOKIE, &config(&s.uri()))
        .await
        .unwrap();
    assert!(!found[0].ready, "expired token is not ready");
    // Malformed input: the error text carries no pasted value.
    let err = complete_manual(
        &store,
        &accounts,
        COOKIE,
        "{\"teams\": nope xoxc-secret-value",
    )
    .await
    .unwrap_err();
    let SignInError::Failed(msg) = err else {
        panic!("expected Failed");
    };
    let msg = format!("{msg:?}");
    assert!(
        !msg.contains("secret-value") && !msg.contains(COOKIE),
        "{msg}"
    );
    let err = complete_manual(
        &store,
        &accounts,
        "not-a-cookie-xoxc-leak",
        &config(&s.uri()),
    )
    .await
    .unwrap_err();
    let SignInError::Failed(msg) = err else {
        panic!("expected Failed");
    };
    let msg = format!("{msg:?}");
    assert!(!msg.contains("leak"), "{msg}");
    assert!(AccountIndex::load(&accounts).unwrap().accounts().is_empty());
}

#[tokio::test]
async fn capture_completion_saves_only_valid_workspaces() {
    let s = server(OK).await;
    let dir = tempfile::tempdir().unwrap();
    let accounts = dir.path().join("accounts.json");
    let store = MemoryStore::default();
    let cookie = slack_api::SessionCookie::parse(COOKIE).unwrap();
    let found = slack_auth::parse_local_config(&config(&s.uri())).unwrap();
    let out = complete_capture(&store, &accounts, cookie, found)
        .await
        .unwrap();
    assert_eq!(out[0].name, "Example");
    assert!(out[0].ready);
}

#[tokio::test]
async fn sign_out_deletes_secrets_and_account() {
    let s = server(OK).await;
    let dir = tempfile::tempdir().unwrap();
    let accounts = dir.path().join("accounts.json");
    let store = MemoryStore::default();
    complete_manual(&store, &accounts, COOKIE, &config(&s.uri()))
        .await
        .unwrap();
    let ws = WorkspaceKey::new("slack", "T0000001");
    sign_out(&store, &accounts, &ws).await.unwrap();
    assert!(CredentialStore::load(&store, &ws).await.unwrap().is_none());
    assert!(AccountIndex::load(&accounts).unwrap().accounts().is_empty());
}

#[test]
fn disallowed_user_agent_switches_to_manual() {
    let e = capture_failure(CaptureFailure::DisallowedUserAgent);
    assert!(matches!(e, SignInError::SwitchTo { ref option, .. } if option == "paste"));
    assert!(matches!(
        capture_failure(CaptureFailure::NoCookie),
        SignInError::Failed(_)
    ));
}

#[test]
fn notice_and_options() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let s = SlackSignIn::new(
        rt.handle().clone(),
        std::path::PathBuf::from("accounts.json"),
        Arc::new(MemoryStore::default()),
    );
    let l = i18n::Localizer::new(
        i18n::chain(
            &i18n::LocalePref::Code("en".parse().unwrap()),
            &[],
            i18n::LocaleSet::embedded(),
        ),
        &[slack::catalog()],
    )
    .unwrap();
    let notice = s
        .notice()
        .iter()
        .map(|t| t.resolve(&l))
        .collect::<Vec<_>>()
        .join("\n");
    for needle in [
        "unofficial",
        "your own slack web session",
        "full access to your account",
        "employer",
        "may change",
        "16",
        "https://oscarbc96.github.io/parla/docs/privacy/",
        "https://oscarbc96.github.io/parla/docs/terms/",
    ] {
        assert!(notice.to_lowercase().contains(needle), "missing {needle}");
    }
    let ids: Vec<String> = s.options().into_iter().map(|o| o.id).collect();
    assert!(ids.contains(&"paste".to_string()));
    let paste = s.options().into_iter().find(|o| o.id == "paste").unwrap();
    assert_eq!(paste.fields.len(), 2);
    assert!(paste.fields.iter().all(|f| f.secret));
    assert_eq!(
        paste.copy.as_deref(),
        Some(slack_auth::MANUAL_CONSOLE_SNIPPET)
    );
}

// The early session path: SessionReady lists workspaces and decides whether to
// open the picker, refresh hidden, or save directly.

struct FakeDriver {
    config: String,
    tx: RefCell<Option<UnboundedSender<Url>>>,
    loads: RefCell<Vec<String>>,
    needs_user: RefCell<bool>,
    /// Team ids that should require the user to sign in.
    needs_user_ids: RefCell<std::collections::HashSet<String>>,
}

impl FakeDriver {
    fn new(config: &str) -> Self {
        Self {
            config: config.to_string(),
            tx: RefCell::new(None),
            loads: RefCell::new(Vec::new()),
            needs_user: RefCell::new(false),
            needs_user_ids: RefCell::new(std::collections::HashSet::new()),
        }
    }
}

impl WebviewDriver for FakeDriver {
    fn eval(&self, _script: &'static str) -> LocalBoxFuture<'_, Result<Option<String>, AuthError>> {
        Box::pin(async move { Ok(Some(self.config.clone())) })
    }

    fn cookies(&self) -> Result<Vec<RawCookie>, AuthError> {
        Ok(vec![RawCookie::new("d", ".slack.com", COOKIE)])
    }

    fn load(&self, url: &Url) -> Result<(), AuthError> {
        self.loads.borrow_mut().push(url.to_string());
        let team = url.path_segments().and_then(|mut s| {
            (s.next() == Some("client"))
                .then(|| s.next().map(str::to_string))
                .flatten()
        });
        let needs_user = *self.needs_user.borrow()
            || team
                .as_ref()
                .is_some_and(|id| self.needs_user_ids.borrow().contains(id));
        if needs_user {
            if let Some(tx) = self.tx.borrow().as_ref() {
                tx.unbounded_send(Url::parse("https://slack.com/sign_in").expect("valid url"))
                    .expect("receiver alive");
            }
            return Ok(());
        }
        if let Some(tx) = self.tx.borrow().as_ref() {
            tx.unbounded_send(url.clone()).expect("receiver alive");
        }
        Ok(())
    }

    fn show_handoff_page(&self, _url: &Url) -> Result<(), AuthError> {
        Ok(())
    }
}

fn listed(id: &str, domain: &str) -> SigninWorkspace {
    SigninWorkspace {
        id: id.into(),
        name: format!("{id} Workspace"),
        domain: domain.into(),
        icon: None,
        sso_required: false,
        two_factor_required: false,
        is_logged_out: false,
    }
}

fn added_choice(id: &str, updating: bool) -> WorkspaceChoice {
    WorkspaceChoice {
        id: id.into(),
        name: format!("{id} Workspace"),
        domain: "one".into(),
        icon: None,
        sso: false,
        two_factor: false,
        signed_out: false,
        added: true,
        updated: false,
        updating,
        update_error: false,
        needs_sign_in: false,
    }
}

async fn seed_added(accounts: &std::path::Path, store: &Arc<MemoryStore>) {
    AccountIndex::update(accounts, |idx| {
        idx.upsert(Account {
            workspace: WorkspaceKey::new("slack", "T0000001"),
            name: "Old".into(),
            url: Url::parse("https://one.slack.com/").unwrap(),
            user_id: "U-old".into(),
            enterprise_id: None,
            enterprise_name: None,
            icon_url: None,
        });
    })
    .unwrap();
    CredentialStore::save(
        store.as_ref(),
        &WorkspaceCredentials {
            workspace: WorkspaceKey::new("slack", "T0000001"),
            user_id: "U-old".into(),
            url: Url::parse("https://one.slack.com/").unwrap(),
            token: XoxcToken::parse("xoxc-old").unwrap(),
            cookie: SessionCookie::parse(COOKIE).unwrap(),
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn early_session_picker_for_new_workspace_starts_added_refresh_in_background() {
    let dir = tempfile::tempdir().unwrap();
    let accounts = dir.path().join("accounts.json");
    let store = Arc::new(MemoryStore::default());
    seed_added(&accounts, &store).await;

    let prepared = early_session(
        &accounts,
        Some(vec![listed("T0000001", "one"), listed("T0000002", "two")]),
    )
    .expect("a listed session opens the picker");

    assert_eq!(prepared.rows.len(), 2);
    assert!(prepared.rows[0].added);
    assert!(prepared.rows[0].updating, "added row starts Updating");
    assert!(!prepared.rows[0].updated);
    assert_eq!(prepared.refresh, vec!["T0000001".to_string()]);
    assert!(!prepared.rows[1].added);
    assert!(!prepared.rows[1].updating);

    // No refresh capture has run yet, so the saved credential is unchanged.
    let saved = CredentialStore::load(store.as_ref(), &WorkspaceKey::new("slack", "T0000001"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.token.expose(), "xoxc-old");
}

#[tokio::test]
async fn added_row_refresh_reports_updated_per_team() {
    let s = server(OK).await;
    let dir = tempfile::tempdir().unwrap();
    let accounts = dir.path().join("accounts.json");
    let store = Arc::new(MemoryStore::default());
    seed_added(&accounts, &store).await;

    let mut model = WorkspacePickerModel::new(vec![added_choice("T0000001", true)]);
    assert!(model.workspaces()[0].updating);

    let driver = FakeDriver::new(&config(&s.uri()));
    let (tx, mut urls) = futures::channel::mpsc::unbounded();
    *driver.tx.borrow_mut() = Some(tx);
    let backend: Arc<dyn CredentialBackend> = store.clone();
    let (progress_tx, mut progress_rx) = futures::channel::mpsc::unbounded();
    slack::sign_in::refresh_teams(
        &driver,
        &mut urls,
        &tokio::runtime::Handle::current(),
        &backend,
        &accounts,
        &format!("d={COOKIE}"),
        &["T0000001".to_string()],
        &progress_tx,
    )
    .await;

    let step = progress_rx.try_recv().expect("refresh outcome");
    assert_eq!(step.id, "T0000001");
    match step.outcome {
        components::workspace_picker::RefreshOutcome::Updated(key) => {
            assert_eq!(key, WorkspaceKey::new("slack", "T0000001"));
            model.refresh_updated(&step.id);
        }
        other => panic!("expected Updated, got {other:?}"),
    }
    assert!(!model.workspaces()[0].updating);
    assert!(model.workspaces()[0].updated);

    let saved = CredentialStore::load(store.as_ref(), &WorkspaceKey::new("slack", "T0000001"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.token.expose(), TOKEN);
}

#[tokio::test]
async fn added_row_refresh_reports_needs_user() {
    let s = server(OK).await;
    let dir = tempfile::tempdir().unwrap();
    let accounts = dir.path().join("accounts.json");
    let store = Arc::new(MemoryStore::default());
    seed_added(&accounts, &store).await;

    let mut model = WorkspacePickerModel::new(vec![added_choice("T0000001", true)]);
    assert!(model.workspaces()[0].updating);

    let driver = FakeDriver::new(&config(&s.uri()));
    driver.needs_user.replace(true);
    let (tx, mut urls) = futures::channel::mpsc::unbounded();
    *driver.tx.borrow_mut() = Some(tx);
    let backend: Arc<dyn CredentialBackend> = store.clone();
    let (progress_tx, mut progress_rx) = futures::channel::mpsc::unbounded();
    slack::sign_in::refresh_teams(
        &driver,
        &mut urls,
        &tokio::runtime::Handle::current(),
        &backend,
        &accounts,
        &format!("d={COOKIE}"),
        &["T0000001".to_string()],
        &progress_tx,
    )
    .await;

    let step = progress_rx.try_recv().expect("refresh outcome");
    assert_eq!(step.id, "T0000001");
    assert!(matches!(
        step.outcome,
        components::workspace_picker::RefreshOutcome::NeedsUser
    ));
    model.refresh_needs_user(&step.id);
    assert!(!model.workspaces()[0].updating);
    assert!(model.workspaces()[0].needs_sign_in);
    assert!(model.selectable("T0000001"));
}

#[tokio::test(flavor = "current_thread")]
async fn refresh_reports_each_updated_before_the_next_team_starts() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let s = server(OK).await;
            let config2 = format!(
                r#"{{"teams":{{"T0000001":{{"name":"Acme","url":"{}/","token":"xoxc-1111-2222-3333-aaaa"}},"T0000002":{{"name":"Beta","url":"{}/","token":"xoxc-4444-5555-6666-bbbb"}}}}}}"#,
                s.uri(),
                s.uri()
            );
            let dir = tempfile::tempdir().unwrap();
            let accounts = dir.path().join("accounts.json");
            let store = Arc::new(MemoryStore::default());
            AccountIndex::update(&accounts, |idx| {
                for (id, user) in [("T0000001", "U-old1"), ("T0000002", "U-old2")] {
                    idx.upsert(Account {
                        workspace: WorkspaceKey::new("slack", id),
                        name: "Old".into(),
                        url: Url::parse(&format!("{}/", s.uri())).unwrap(),
                        user_id: user.into(),
                        enterprise_id: None,
                        enterprise_name: None,
                        icon_url: None,
                    });
                }
            })
            .unwrap();
            for id in ["T0000001", "T0000002"] {
                CredentialStore::save(
                    store.as_ref(),
                    &WorkspaceCredentials {
                        workspace: WorkspaceKey::new("slack", id),
                        user_id: "U-old".into(),
                        url: Url::parse(&format!("{}/", s.uri())).unwrap(),
                        token: XoxcToken::parse("xoxc-old").unwrap(),
                        cookie: SessionCookie::parse(COOKIE).unwrap(),
                    },
                )
                .await
                .unwrap();
            }

            struct SeqDriver {
                config: String,
                tx: RefCell<Option<UnboundedSender<Url>>>,
                loads: Rc<RefCell<Vec<String>>>,
                auto_emit: RefCell<bool>,
            }
            impl WebviewDriver for SeqDriver {
                fn eval(
                    &self,
                    _script: &'static str,
                ) -> LocalBoxFuture<'_, Result<Option<String>, AuthError>> {
                    let cfg = self.config.clone();
                    Box::pin(async move { Ok(Some(cfg)) })
                }
                fn cookies(&self) -> Result<Vec<RawCookie>, AuthError> {
                    Ok(vec![RawCookie::new("d", ".slack.com", COOKIE)])
                }
                fn load(&self, url: &Url) -> Result<(), AuthError> {
                    self.loads.borrow_mut().push(url.to_string());
                    if *self.auto_emit.borrow()
                        && let Some(tx) = self.tx.borrow().as_ref()
                    {
                        tx.unbounded_send(url.clone()).expect("receiver alive");
                    }
                    Ok(())
                }
                fn show_handoff_page(&self, _url: &Url) -> Result<(), AuthError> {
                    Ok(())
                }
            }

            let loads = Rc::new(RefCell::new(Vec::new()));
            let (tx, urls) = futures::channel::mpsc::unbounded();
            let driver = Rc::new(SeqDriver {
                config: config2,
                tx: RefCell::new(Some(tx.clone())),
                loads: loads.clone(),
                auto_emit: RefCell::new(false),
            });
            let backend: Arc<dyn CredentialBackend> = store.clone();
            let (progress_tx, mut progress_rx) = futures::channel::mpsc::unbounded();
            let path = accounts.clone();
            let driver_task = driver.clone();
            let refresh = tokio::task::spawn_local(async move {
                let mut urls = urls;
                let driver_ref: &SeqDriver = &driver_task;
                slack::sign_in::refresh_teams(
                    driver_ref,
                    &mut urls,
                    &tokio::runtime::Handle::current(),
                    &backend,
                    &path,
                    &format!("d={COOKIE}"),
                    &["T0000001".to_string(), "T0000002".to_string()],
                    &progress_tx,
                )
                .await;
            });

            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            assert_eq!(
                *loads.borrow(),
                vec!["https://app.slack.com/client/T0000001".to_string()],
                "T1 started first"
            );

            tx.unbounded_send(Url::parse("https://app.slack.com/client/T0000001").unwrap())
                .unwrap();
            let step = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                futures::StreamExt::next(&mut progress_rx),
            )
            .await
            .expect("T1 outcome timeout")
            .expect("T1 outcome");
            assert_eq!(step.id, "T0000001");
            assert!(
                matches!(
                    step.outcome,
                    components::workspace_picker::RefreshOutcome::Updated(_)
                ),
                "T1 outcome: {:?}",
                step.outcome
            );
            // T1's Updated is reported before refresh_teams moves on to T2.
            assert!(
                !loads
                    .borrow()
                    .iter()
                    .any(|u| u == "https://app.slack.com/client/T0000002"),
                "T1 Updated arrived before T2 started"
            );

            let t2 = Url::parse("https://app.slack.com/client/T0000002").unwrap();
            for _ in 0..50 {
                tokio::task::yield_now().await;
                if loads
                    .borrow()
                    .iter()
                    .any(|u| u == "https://app.slack.com/client/T0000002")
                {
                    break;
                }
            }
            assert!(
                loads
                    .borrow()
                    .iter()
                    .any(|u| u == "https://app.slack.com/client/T0000002"),
                "T2 started"
            );
            tx.unbounded_send(t2).unwrap();
            let step = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                futures::StreamExt::next(&mut progress_rx),
            )
            .await
            .expect("T2 outcome timeout")
            .expect("T2 outcome");
            assert_eq!(step.id, "T0000002");
            let _ = refresh.await;
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn add_queues_behind_a_running_refresh() {
    use slack_auth::webview::AdderWebview;

    struct TestDriver {
        config: String,
        tx: RefCell<Option<UnboundedSender<Url>>>,
        loads: Rc<RefCell<Vec<String>>>,
        auto_emit: RefCell<bool>,
    }

    impl WebviewDriver for TestDriver {
        fn eval(
            &self,
            _script: &'static str,
        ) -> LocalBoxFuture<'_, Result<Option<String>, AuthError>> {
            let cfg = self.config.clone();
            Box::pin(async move { Ok(Some(cfg)) })
        }

        fn cookies(&self) -> Result<Vec<RawCookie>, AuthError> {
            Ok(vec![RawCookie::new("d", ".slack.com", COOKIE)])
        }

        fn load(&self, url: &Url) -> Result<(), AuthError> {
            self.loads.borrow_mut().push(url.to_string());
            if *self.auto_emit.borrow()
                && let Some(tx) = self.tx.borrow().as_ref()
            {
                tx.unbounded_send(url.clone()).expect("receiver alive");
            }
            Ok(())
        }

        fn show_handoff_page(&self, _url: &Url) -> Result<(), AuthError> {
            Ok(())
        }
    }

    impl WebviewDriver for &TestDriver {
        fn eval(
            &self,
            script: &'static str,
        ) -> LocalBoxFuture<'_, Result<Option<String>, AuthError>> {
            (**self).eval(script)
        }

        fn cookies(&self) -> Result<Vec<RawCookie>, AuthError> {
            (**self).cookies()
        }

        fn load(&self, url: &Url) -> Result<(), AuthError> {
            (**self).load(url)
        }

        fn show_handoff_page(&self, url: &Url) -> Result<(), AuthError> {
            (**self).show_handoff_page(url)
        }
    }

    struct TestWebview {
        driver: TestDriver,
    }

    impl AdderWebview for TestWebview {
        type Driver<'a> = &'a TestDriver;

        fn driver(&self) -> Self::Driver<'_> {
            &self.driver
        }

        fn set_visible(&self, _visible: bool) -> Result<(), AuthError> {
            Ok(())
        }

        fn sign_in_done(&self) {}
    }

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let s = server(OK).await;
            let config2 = format!(
                r#"{{"teams":{{"T0000001":{{"name":"Acme","url":"{0}/","token":"xoxc-1111-2222-3333-aaaa"}},"T0000002":{{"name":"Beta","url":"{0}/","token":"xoxc-4444-5555-6666-bbbb"}}}}}}"#,
                s.uri()
            );
            let dir = tempfile::tempdir().unwrap();
            let accounts = dir.path().join("accounts.json");
            let store = Arc::new(MemoryStore::default());
            seed_added(&accounts, &store).await;

            let loads = Rc::new(RefCell::new(Vec::new()));
            let (tx, urls) = futures::channel::mpsc::unbounded();
            let webview = Rc::new(TestWebview {
                driver: TestDriver {
                    config: config2,
                    tx: RefCell::new(Some(tx.clone())),
                    loads: loads.clone(),
                    auto_emit: RefCell::new(false),
                },
            });

            let backend: Arc<dyn CredentialBackend> = store.clone();
            let adder = Rc::new(slack::sign_in::SlackAdder::new(
                tokio::runtime::Handle::current(),
                backend,
                accounts.clone(),
                webview.clone(),
                urls,
                zeroize::Zeroizing::new(format!("d={COOKIE}")),
            ));

            let (refresh_tx, mut refresh_rx) = futures::channel::mpsc::unbounded();
            let adder_refresh = adder.clone();
            let refresh = tokio::task::spawn_local(async move {
                adder_refresh
                    .run_refresh(vec!["T0000001".to_string()], refresh_tx)
                    .await;
            });

            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            assert_eq!(
                *loads.borrow(),
                vec!["https://app.slack.com/client/T0000001".to_string()],
                "refresh started"
            );

            let (add_tx, _add_rx) = futures::channel::mpsc::unbounded();
            let adder_add = adder.clone();
            let add = tokio::task::spawn_local(async move {
                adder_add
                    .run_add(vec!["T0000002".to_string()], add_tx)
                    .await
            });

            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            assert_eq!(
                loads.borrow().len(),
                1,
                "add must wait for refresh: no add navigation before refresh's last result"
            );

            webview.driver.auto_emit.replace(true);
            tx.unbounded_send(Url::parse("https://app.slack.com/client/T0000001").unwrap())
                .unwrap();
            let step = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                futures::StreamExt::next(&mut refresh_rx),
            )
            .await
            .expect("refresh outcome timeout")
            .expect("refresh outcome");
            assert_eq!(step.id, "T0000001");
            refresh.await.unwrap();

            let result = tokio::time::timeout(std::time::Duration::from_secs(5), add)
                .await
                .expect("add timeout")
                .unwrap()
                .expect("add result");
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].key, WorkspaceKey::new("slack", "T0000002"));
            assert!(result[0].ready);

            let saved =
                CredentialStore::load(store.as_ref(), &WorkspaceKey::new("slack", "T0000002"))
                    .await
                    .unwrap();
            assert!(saved.is_some(), "added workspace saved");
            let idx = AccountIndex::load(&accounts).unwrap();
            assert!(
                idx.accounts()
                    .iter()
                    .any(|a| a.workspace == WorkspaceKey::new("slack", "T0000002")),
                "added workspace in account index"
            );
        })
        .await;
}

#[tokio::test]
async fn refresh_reports_the_refreshed_key_without_a_pick() {
    // Equivalent to main: a refreshed added workspace yields its key without
    // the user picking a new workspace (the picker reports it on cancel/close).
    let s = server(OK).await;
    let dir = tempfile::tempdir().unwrap();
    let accounts = dir.path().join("accounts.json");
    let store = Arc::new(MemoryStore::default());
    seed_added(&accounts, &store).await;

    let driver = FakeDriver::new(&config(&s.uri()));
    let (tx, mut urls) = futures::channel::mpsc::unbounded();
    *driver.tx.borrow_mut() = Some(tx);
    let backend: Arc<dyn CredentialBackend> = store.clone();
    let (progress_tx, mut progress_rx) = futures::channel::mpsc::unbounded();
    slack::sign_in::refresh_teams(
        &driver,
        &mut urls,
        &tokio::runtime::Handle::current(),
        &backend,
        &accounts,
        &format!("d={COOKIE}"),
        &["T0000001".to_string()],
        &progress_tx,
    )
    .await;

    let step = progress_rx.try_recv().expect("refresh outcome");
    assert!(
        matches!(
            step.outcome,
            components::workspace_picker::RefreshOutcome::Updated(ref key)
                if *key == WorkspaceKey::new("slack", "T0000001")
        ),
        "refreshed key reported without a pick"
    );
}
