use futures::future::LocalBoxFuture;
use futures::stream;
use slack_auth::webview::testing::{CONFIG, FakeWebview as Fake, dcookie, scripted};
use slack_auth::webview::{
    CaptureFailure, CaptureState, NoLookup, RawCookie, TeamLookup, UrlKind, classify_url,
    cookie_from_jar, decode_eval, run_capture, run_capture_with,
};
use slack_auth::{MemoryStore, SessionStatus, persist_verified};
use url::Url;

const D: &str = "xoxd-AbCdEf%2Bghi";

fn u(s: &str) -> Url {
    Url::parse(s).unwrap()
}

#[tokio::test]
async fn client_page_with_cookie_is_session_ready() {
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    let urls = stream::iter(vec![
        u("https://app.slack.com/signin"),
        u("https://app.slack.com/client/T0000001/C0000001"),
    ]);
    match run_capture(&fake, urls).await {
        CaptureState::SessionReady(cookie) => {
            assert!(cookie.header_value().starts_with("d=xoxd-"));
        }
        other => panic!("expected SessionReady, got {other:?}"),
    }
    // The session is listed before the team is read.
    assert_eq!(*fake.evals.borrow(), 0);
}

#[tokio::test]
async fn signin_page_with_cookie_is_session_ready() {
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    let urls = stream::iter(vec![u("https://slack.com/signin#/signin")]);
    match run_capture(&fake, urls).await {
        CaptureState::SessionReady(cookie) => {
            assert!(cookie.header_value().starts_with("d=xoxd-"));
        }
        other => panic!("expected SessionReady, got {other:?}"),
    }
    // The session page is enough: no client page was read.
    assert_eq!(*fake.evals.borrow(), 0);
}

#[tokio::test]
async fn workspace_signin_page_with_cookie_is_session_ready() {
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    let urls = stream::iter(vec![u("https://slack.com/workspace-signin")]);
    assert!(matches!(
        run_capture(&fake, urls).await,
        CaptureState::SessionReady(..)
    ));
    assert_eq!(*fake.evals.borrow(), 0);
}

#[tokio::test]
async fn app_signin_page_with_cookie_is_session_ready() {
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    let urls = stream::iter(vec![u("https://app.slack.com/signin#/workspaces")]);
    match run_capture(&fake, urls).await {
        CaptureState::SessionReady(cookie) => {
            assert!(cookie.header_value().starts_with("d=xoxd-"));
        }
        other => panic!("expected SessionReady, got {other:?}"),
    }
    assert_eq!(*fake.evals.borrow(), 0);
}

#[tokio::test]
async fn signin_page_cookie_arriving_on_later_same_path_event_is_session_ready() {
    struct CookieLater(std::cell::Cell<bool>);
    impl slack_auth::webview::WebviewDriver for CookieLater {
        fn eval(
            &self,
            _script: &'static str,
        ) -> LocalBoxFuture<'_, Result<Option<String>, slack_auth::AuthError>> {
            Box::pin(async { Ok(None) })
        }
        fn cookies(&self) -> Result<Vec<RawCookie>, slack_auth::AuthError> {
            Ok(if self.0.get() {
                vec![dcookie()]
            } else {
                self.0.set(true);
                vec![]
            })
        }
        fn load(&self, _url: &Url) -> Result<(), slack_auth::AuthError> {
            Ok(())
        }
        fn show_handoff_page(&self, _url: &Url) -> Result<(), slack_auth::AuthError> {
            Ok(())
        }
    }
    let driver = CookieLater(std::cell::Cell::new(false));
    let urls = stream::iter(vec![
        u("https://slack.com/signin"),
        u("https://slack.com/signin#/workspaces"),
    ]);
    assert!(matches!(
        run_capture(&driver, urls).await,
        CaptureState::SessionReady(..)
    ));
}

#[tokio::test]
async fn signin_page_without_cookie_keeps_waiting() {
    let fake = Fake::new(Some(CONFIG), vec![]);
    let urls = stream::iter(vec![u("https://slack.com/signin?x=1")]);
    assert!(matches!(
        run_capture(&fake, urls).await,
        CaptureState::Failed(CaptureFailure::Closed)
    ));
    assert_eq!(*fake.evals.borrow(), 0);
}

#[test]
fn bare_and_app_slack_signin_pages_are_session_pages() {
    assert_eq!(
        classify_url(&u("https://slack.com/signin")),
        UrlKind::SessionPage
    );
    assert_eq!(
        classify_url(&u("https://slack.com/signin#/signin")),
        UrlKind::SessionPage
    );
    assert_eq!(
        classify_url(&u("https://slack.com/signin?x=1")),
        UrlKind::SessionPage
    );
    assert_eq!(
        classify_url(&u("https://slack.com/workspace-signin")),
        UrlKind::SessionPage
    );
    assert_eq!(
        classify_url(&u("https://app.slack.com/signin")),
        UrlKind::SessionPage
    );
    assert_eq!(
        classify_url(&u("https://app.slack.com/signin#/workspaces")),
        UrlKind::SessionPage
    );
    assert_eq!(
        classify_url(&u("https://app.slack.com/workspace-signin")),
        UrlKind::SessionPage
    );
    // Workspace hosts keep signing in.
    assert_eq!(
        classify_url(&u("https://acme.slack.com/signin")),
        UrlKind::SigningIn
    );
}

#[tokio::test]
async fn missing_cookie_fails_cleanly() {
    let other = RawCookie::new("b", ".slack.com", "x");
    let fake = Fake::new(Some(CONFIG), vec![other]);
    let urls = stream::iter(vec![u("https://app.slack.com/client/T0000001/C0000001")]);
    assert!(matches!(
        run_capture(&fake, urls).await,
        CaptureState::Failed(CaptureFailure::NoCookie)
    ));
}

#[tokio::test]
async fn no_teams_and_closed() {
    let fake = Fake::new(None, vec![dcookie()]);
    let urls = stream::iter(vec![u("https://app.slack.com/client/T0000001/C0000001")]);
    // The session is listed first; the caller falls back to reading this team if
    // the listing fails.
    assert!(matches!(
        run_capture(&fake, urls).await,
        CaptureState::SessionReady(..)
    ));
    let urls = stream::iter(vec![u("https://app.slack.com/")]);
    assert!(matches!(
        run_capture(&fake, urls).await,
        CaptureState::Failed(CaptureFailure::Closed)
    ));
}

#[tokio::test]
async fn google_disallowed_user_agent_detected() {
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    let urls = stream::iter(vec![u(
        "https://accounts.google.com/signin/oauth/error?authError=x&flowName=GeneralOAuthFlow&disallowed_useragent=1",
    )]);
    assert!(matches!(
        run_capture(&fake, urls).await,
        CaptureState::Failed(CaptureFailure::DisallowedUserAgent)
    ));
    assert_eq!(
        classify_url(&u("https://app.slack.com/signin")),
        UrlKind::SessionPage
    );
}

#[test]
fn cookie_filter_uses_domain_suffix() {
    let cs = vec![
        RawCookie::new("d", ".evilslack.com", "xoxd-bad"),
        RawCookie::new("d", ".slack.com", D),
        RawCookie::new("d-s", ".slack.com", "1700000000"),
    ];
    let c = cookie_from_jar(&cs).unwrap();
    let h = c.header_value();
    assert!(h.contains("d=xoxd-AbCdEf") && h.contains("d-s=1700000000"));
    assert!(!h.contains("bad"));
    assert!(cookie_from_jar(&[RawCookie::new("d", "app.slack.com", D)]).is_some());
    assert!(cookie_from_jar(&[RawCookie::new("d", "example.com", D)]).is_none());
    assert!(format!("{:?}", cs[1]).find("AbCdEf").is_none());
}

#[test]
fn eval_result_is_json_string_literal() {
    assert_eq!(decode_eval("null"), None);
    assert_eq!(decode_eval(r#""{\"a\":1}""#).as_deref(), Some(r#"{"a":1}"#));
    assert_eq!(decode_eval("garbage"), None);
}

#[tokio::test]
async fn persists_only_valid_workspaces() {
    let ws = slack_auth::parse_local_config(CONFIG).unwrap();
    let cookie = slack_api::SessionCookie::parse(D).unwrap();
    let key = ws[0].workspace.clone();
    let verified = vec![(
        ws.into_iter().next().unwrap(),
        SessionStatus::Valid {
            user_id: "U0000001".into(),
            team_name: "Acme".into(),
        },
    )];
    let store = MemoryStore::default();
    let saved = persist_verified(&store, &cookie, verified).await.unwrap();
    assert_eq!(saved, vec![key.clone()]);
    use slack_auth::CredentialStore;
    assert!(store.load(&key).await.unwrap().is_some());
}

struct Lookup(Option<&'static str>, std::cell::Cell<u32>);

impl TeamLookup for Lookup {
    fn team_id(
        &self,
        domain: &str,
        _: &slack_api::SessionCookie,
    ) -> LocalBoxFuture<'_, Option<String>> {
        assert_eq!(domain, "acme");
        self.1.set(self.1.get() + 1);
        let id = self.0.map(String::from);
        Box::pin(async move { id })
    }
}

#[tokio::test]
async fn redirect_page_with_cookie_is_rewritten_and_session_ready() {
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    let urls = stream::iter(vec![
        u("https://acme.slack.com/ssb/redirect"),
        u("https://app.slack.com/client/T0000001"),
    ]);
    let state = run_capture_with(
        &fake,
        urls,
        &Lookup(Some("T0000001"), Default::default()),
        &mut std::collections::HashSet::new(),
    )
    .await;
    assert!(matches!(state, CaptureState::SessionReady(..)), "{state:?}");
    // The handoff is still rewritten so the flow can continue if the listing fails.
    assert_eq!(
        *fake.loads.borrow(),
        vec!["https://app.slack.com/client/T0000001".to_string()]
    );
}

#[tokio::test]
async fn deep_link_handoff_with_cookie_is_session_ready() {
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    let urls = stream::iter(vec![
        u("slack://T0000001/magic-login/x"),
        u("https://app.slack.com/client/T0000001"),
    ]);
    let state = run_capture_with(
        &fake,
        urls,
        &NoLookup,
        &mut std::collections::HashSet::new(),
    )
    .await;
    assert!(matches!(state, CaptureState::SessionReady(..)), "{state:?}");
    assert_eq!(fake.loads.borrow().len(), 1);
}

#[tokio::test]
async fn unresolved_redirect_with_cookie_shows_page_and_is_session_ready() {
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    let urls = stream::iter(vec![u("https://acme.slack.com/ssb/redirect")]);
    let state = run_capture_with(
        &fake,
        urls,
        &Lookup(None, Default::default()),
        &mut std::collections::HashSet::new(),
    )
    .await;
    assert!(matches!(state, CaptureState::SessionReady(..)), "{state:?}");
    // The page is still shown as is so the user can continue if the listing fails.
    assert_eq!(
        *fake.loads.borrow(),
        vec!["page:https://acme.slack.com/ssb/redirect".to_string()]
    );
}

#[tokio::test]
async fn fallback_page_event_is_not_looked_up_again() {
    let page = "https://acme.slack.com/ssb/redirect";
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    // Showing the page makes the webview report the same URL again.
    let rx = scripted(&fake, vec![(&format!("page:{page}"), vec![page])]);
    fake.tx
        .borrow()
        .as_ref()
        .unwrap()
        .unbounded_send(u(page))
        .unwrap();
    let lookup = Lookup(None, Default::default());
    let state = run_capture_with(&fake, rx, &lookup, &mut std::collections::HashSet::new()).await;
    assert!(matches!(state, CaptureState::SessionReady(..)), "{state:?}");
    assert_eq!(lookup.1.get(), 1, "looked up once");
    assert_eq!(fake.loads.borrow().len(), 1, "shown once");
}

#[tokio::test]
async fn server_team_id_is_validated_before_it_is_used_in_a_url() {
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    let urls = stream::iter(vec![u("https://acme.slack.com/ssb/redirect")]);
    let state = run_capture_with(
        &fake,
        urls,
        &Lookup(Some("T1/../x?y"), Default::default()),
        &mut std::collections::HashSet::new(),
    )
    .await;
    assert!(matches!(state, CaptureState::SessionReady(..)), "{state:?}");
    assert!(fake.loads.borrow().iter().all(|l| l.starts_with("page:")));
}

#[tokio::test]
async fn repeated_handoff_after_failed_listing_does_not_loop() {
    // A handoff with a session cookie returns SessionReady. When the listing and
    // the client read both fail, the caller runs capture again: the shown page's
    // repeated navigation must not be handled again (bounded by event count).
    let page = "https://acme.slack.com/ssb/redirect";
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    let (tx, rx) = futures::channel::mpsc::unbounded();
    *fake.tx.borrow_mut() = Some(tx.clone());
    // The page is shown once and navigates back to itself once.
    *fake.script.borrow_mut() = vec![(format!("page:{page}"), vec![page])];
    tx.unbounded_send(u(page)).unwrap();

    let lookup = Lookup(None, Default::default());
    let mut fell_back = std::collections::HashSet::new();
    let mut urls = rx;

    let state = run_capture_with(&fake, &mut urls, &lookup, &mut fell_back).await;
    assert!(matches!(state, CaptureState::SessionReady(..)), "{state:?}");
    assert_eq!(lookup.1.get(), 1);
    assert_eq!(fake.loads.borrow().len(), 1);

    // No further listing/client read succeeded; the caller retries. The replayed
    // handoff event is ignored, so no second lookup or load happens.
    fake.tx.borrow_mut().take();
    drop(tx);
    let state = run_capture_with(&fake, &mut urls, &lookup, &mut fell_back).await;
    assert!(
        matches!(state, CaptureState::Failed(CaptureFailure::Closed)),
        "{state:?}"
    );
    assert_eq!(lookup.1.get(), 1, "handled once");
    assert_eq!(fake.loads.borrow().len(), 1, "shown once");
}
