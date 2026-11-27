use futures::stream;
use slack_auth::webview::testing::{
    C1, C2, CONFIG2, FakeWebview as Fake, dcookie, no_skip, scripted,
};
use slack_auth::webview::{Start, TeamOutcome, capture_team, capture_teams};
use url::Url;

fn u(s: &str) -> Url {
    Url::parse(s).unwrap()
}

#[tokio::test]
async fn captures_each_team_in_order_and_ignores_stale_events() {
    let fake = Fake::new(Some(CONFIG2), vec![dcookie()]);
    let mut urls = scripted(
        &fake,
        vec![
            (C1, vec![C1, "https://app.slack.com/client/T0000001/C1"]),
            (C2, vec![C2]),
        ],
    );
    let ids = vec!["T0000001".to_string(), "T0000002".to_string()];
    let mut seen = Vec::new();
    let res = capture_teams(&fake, &mut urls, &mut no_skip(), &ids, |id, o| {
        seen.push((id.to_string(), matches!(o, TeamOutcome::Captured(_))));
        true
    })
    .await;
    assert_eq!(res.len(), 2);
    assert!(
        res.iter()
            .all(|r| matches!(r.outcome, TeamOutcome::Captured(_)))
    );
    // The leftover T1 event is drained before T2 starts, so T2 is not confused by it.
    assert_eq!(*fake.loads.borrow(), vec![C1.to_string(), C2.to_string()]);
    assert_eq!(seen.len(), 2);
}

#[tokio::test]
async fn leaving_the_client_needs_the_user() {
    let fake = Fake::new(Some(CONFIG2), vec![dcookie()]);
    let mut urls = scripted(
        &fake,
        vec![(C1, vec!["https://acme.slack.com/sso/saml/start"])],
    );
    let out = capture_team(&fake, &mut urls, "T0000001", Start::Navigate).await;
    assert!(matches!(out, TeamOutcome::NeedsUser));
    assert_eq!(*fake.loads.borrow(), vec![C1.to_string()]);
}

#[tokio::test]
async fn resume_waits_for_the_client_without_navigating() {
    let fake = Fake::new(Some(CONFIG2), vec![dcookie()]);
    let mut urls = stream::iter(vec![u("https://app.slack.com/client/T0000002")]);
    let out = capture_team(&fake, &mut urls, "T0000002", Start::Resume).await;
    assert!(matches!(out, TeamOutcome::Captured(_)));
    assert!(fake.loads.borrow().is_empty());
}

#[tokio::test]
async fn skipped_team_stays_needs_user_and_next_team_still_runs() {
    let fake = Fake::new(Some(CONFIG2), vec![dcookie()]);
    let mut urls = scripted(
        &fake,
        vec![
            (C1, vec!["https://acme.slack.com/sso/saml/start"]),
            (C2, vec![C2]),
        ],
    );
    let ids = vec!["T0000001".to_string(), "T0000002".to_string()];
    let res = capture_teams(&fake, &mut urls, &mut no_skip(), &ids, |_, _| false).await;
    assert!(matches!(res[0].outcome, TeamOutcome::NeedsUser));
    assert!(matches!(res[1].outcome, TeamOutcome::Captured(_)));
}

#[tokio::test]
async fn invalid_team_id_is_not_navigated() {
    let fake = Fake::new(Some(CONFIG2), vec![dcookie()]);
    let mut urls = stream::iter(Vec::<Url>::new());
    let out = capture_team(&fake, &mut urls, "T1/../x", Start::Navigate).await;
    assert!(matches!(out, TeamOutcome::Failed(_)));
    assert!(fake.loads.borrow().is_empty());
}

#[tokio::test]
async fn skipped_team_is_reported_once() {
    let fake = Fake::new(Some(CONFIG2), vec![dcookie()]);
    let mut urls = scripted(
        &fake,
        vec![(C1, vec!["https://acme.slack.com/sso/saml/start"])],
    );
    let ids = vec!["T0000001".to_string()];
    let mut calls = 0;
    capture_teams(&fake, &mut urls, &mut no_skip(), &ids, |_, _| {
        calls += 1;
        false
    })
    .await;
    assert_eq!(calls, 1);
}

#[tokio::test]
async fn resume_ignores_sign_in_pages_until_the_teams_client() {
    let fake = Fake::new(Some(CONFIG2), vec![dcookie()]);
    let mut urls = stream::iter(vec![
        u("https://beta.slack.com/sso/saml/start"),
        u("https://login.idp.example/mfa"),
        u("https://slack.com/signin"),
        u("https://app.slack.com/client/T0000001"),
        u(C2),
    ]);
    let out = capture_team(&fake, &mut urls, "T0000002", Start::Resume).await;
    assert!(matches!(out, TeamOutcome::Captured(_)));
}

#[tokio::test]
async fn resuming_through_sign_in_pages_captures_the_team_in_a_run() {
    let fake = Fake::new(Some(CONFIG2), vec![dcookie()]);
    let mut urls = scripted(
        &fake,
        vec![(
            C2,
            vec![
                "https://beta.slack.com/sso/saml/start",
                "https://login.idp.example/mfa",
                C2,
            ],
        )],
    );
    let ids = vec!["T0000002".to_string()];
    let mut seen = Vec::new();
    let res = capture_teams(&fake, &mut urls, &mut no_skip(), &ids, |_, o| {
        seen.push(matches!(o, TeamOutcome::NeedsUser));
        true
    })
    .await;
    assert!(matches!(res[0].outcome, TeamOutcome::Captured(_)));
    // NeedsUser once, then the final outcome.
    assert_eq!(seen, vec![true, false]);
}

#[tokio::test]
async fn skip_abandons_the_waiting_team_and_the_next_one_runs() {
    let fake = Fake::new(Some(CONFIG2), vec![dcookie()]);
    let mut urls = scripted(
        &fake,
        vec![
            (C1, vec!["https://acme.slack.com/sso/saml/start"]),
            (C2, vec![C2]),
        ],
    );
    // Pending for the stale-click drain, then the click arrives while waiting.
    let mut polls = 0;
    let mut skip = stream::poll_fn(move |cx| {
        polls += 1;
        if polls < 2 {
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(Some(()))
        }
    });
    let ids = vec!["T0000001".to_string(), "T0000002".to_string()];
    let res = capture_teams(&fake, &mut urls, &mut skip, &ids, |_, _| true).await;
    assert!(matches!(res[0].outcome, TeamOutcome::NeedsUser));
    assert!(matches!(res[1].outcome, TeamOutcome::Captured(_)));
}

#[tokio::test]
async fn resume_opens_the_client_when_sign_in_ends_on_the_desktop_handoff() {
    // The user finished signing in; Slack ends on "open the desktop app", which
    // is never loaded. The team's client must be opened, or the capture waits
    // for ever and the picker stays on "Signing in".
    let fake = Fake::new(Some(CONFIG2), vec![dcookie()]);
    let mut urls = scripted(&fake, vec![(C2, vec![C2])]);
    fake.tx
        .borrow()
        .as_ref()
        .unwrap()
        .unbounded_send(u("https://beta.slack.com/signin/redirect"))
        .unwrap();
    fake.tx
        .borrow()
        .as_ref()
        .unwrap()
        .unbounded_send(u("slack://T0000002/"))
        .unwrap();
    // A second handoff (a repeated event) must not load the client again.
    fake.tx
        .borrow()
        .as_ref()
        .unwrap()
        .unbounded_send(u("slack://T0000002/"))
        .unwrap();
    // Without the fix the capture waits for ever: fail fast instead.
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        capture_team(&fake, &mut urls, "T0000002", Start::Resume),
    )
    .await
    .expect("the resumed capture finished");
    assert!(matches!(out, TeamOutcome::Captured(_)), "{out:?}");
    assert_eq!(
        *fake.loads.borrow(),
        vec![C2.to_string()],
        "the team client is loaded once"
    );
}
