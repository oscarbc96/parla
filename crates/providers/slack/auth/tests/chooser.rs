//! Signing in from Slack's chooser after the email code, before any session cookie.

use futures::stream;
use slack_auth::webview::testing::{C1, CONFIG, CONFIG2, FakeWebview as Fake, dcookie, scripted};
use slack_auth::webview::{
    CHOOSER_HOOK_SCRIPT, CHOOSER_PREFIX, CaptureState, Start, TeamOutcome, capture_team,
    chooser_list, run_capture,
};
use url::Url;

const FROM: &str = "https://app.slack.com/signin";
const LINK1: &str = "https://acme.slack.com/z-app-1-abc";

fn ws(id: &str) -> slack_api::SigninWorkspace {
    slack_api::SigninWorkspace {
        id: id.into(),
        name: "Acme".into(),
        domain: "acme".into(),
        icon: None,
        sso_required: false,
        two_factor_required: false,
        is_logged_out: false,
    }
}

fn msg(json: &str) -> String {
    format!("{CHOOSER_PREFIX}{json}")
}

#[test]
fn chooser_list_keeps_teams_and_links() {
    let body = msg(
        r#"[{"id":"T0000001","name":"Acme","domain":"acme","icon":"https://i/1.png",
            "sso_required":true,"link":"https://acme.slack.com/z-app-1-abc"},
           {"id":"T0000002","name":"Beta","domain":"beta","icon":"http://i/2.png",
            "link":"https://beta.slack.com/z-app-2-def"}]"#,
    );
    let list = chooser_list(FROM, &body).unwrap();
    let ids: Vec<_> = list.teams.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, ["T0000001", "T0000002"]);
    assert!(list.teams[0].sso_required);
    assert_eq!(list.teams[0].icon.as_deref(), Some("https://i/1.png"));
    assert_eq!(list.teams[1].icon, None, "only https icons");
    assert_eq!(list.links["T0000001"].as_str(), LINK1);
    let dbg = format!("{list:?}");
    assert!(!dbg.contains("z-app"), "links are redacted in Debug: {dbg}");
}

#[test]
fn chooser_list_drops_bad_ids_and_links() {
    let body = msg(r#"[{"id":"T1/../x","link":"https://acme.slack.com/z"},
           {"id":"T0000002","link":"https://evil.example/z"},
           {"id":"T0000003","link":"http://acme.slack.com/z"},
           {"id":"T0000004"},
           {"id":"T0000005","link":"https://five.slack.com/z"},
           {"id":"T0000005","link":"https://again.slack.com/z"}]"#);
    let list = chooser_list(FROM, &body).unwrap();
    let ids: Vec<_> = list.teams.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, ["T0000005"]);
    assert_eq!(list.links["T0000005"].as_str(), "https://five.slack.com/z");
}

#[test]
fn chooser_list_only_from_slack_frames() {
    let body = msg(r#"[{"id":"T0000001","link":"https://acme.slack.com/z"}]"#);
    assert!(chooser_list("https://slack.com/signin", &body).is_some());
    assert!(chooser_list("https://ads.example/frame", &body).is_none());
    assert!(chooser_list("https://acme.slack.com/sso", &body).is_none());
    assert!(chooser_list("http://app.slack.com/signin", &body).is_none());
    assert!(
        chooser_list(FROM, r#"[{"id":"T0000001"}]"#).is_none(),
        "needs the prefix"
    );
    assert!(chooser_list(FROM, &msg("not json")).is_none());
    assert!(chooser_list(FROM, &msg("[]")).is_none());
}

#[test]
fn hook_script_reads_the_chooser_answer() {
    let s = CHOOSER_HOOK_SCRIPT;
    assert!(s.contains("/api/signin.findWorkspaces"));
    assert!(s.contains("magic_login_url"));
    assert!(s.contains(CHOOSER_PREFIX));
    assert!(s.contains("window.fetch") && s.contains("XMLHttpRequest.prototype.open"));
    assert!(
        s.contains("postMessage(location.href)"),
        "wakes the capture"
    );
}

#[tokio::test]
async fn chooser_without_cookie_is_reported() {
    let fake = Fake::new(None, vec![]);
    *fake.chooser.borrow_mut() = Some(vec![ws("T0000001")]);
    let urls = stream::iter(vec![
        Url::parse("https://app.slack.com/signin#/workspaces").unwrap(),
    ]);
    match run_capture(&fake, urls).await {
        CaptureState::Chooser(list) => assert_eq!(list[0].id, "T0000001"),
        other => panic!("expected Chooser, got {other:?}"),
    }
}

#[tokio::test]
async fn session_page_without_cookie_or_chooser_keeps_waiting() {
    let fake = Fake::new(None, vec![]);
    let urls = stream::iter(vec![
        Url::parse("https://app.slack.com/signin#/workspaces").unwrap(),
    ]);
    assert!(matches!(
        run_capture(&fake, urls).await,
        CaptureState::Failed(_)
    ));
}

#[tokio::test]
async fn login_link_through_the_handoff_reaches_the_client() {
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    fake.links
        .borrow_mut()
        .insert("T0000001".into(), LINK1.into());
    let mut urls = scripted(
        &fake,
        vec![
            (
                LINK1,
                vec![
                    "https://acme.slack.com/z-app-1-abc",
                    "https://acme.slack.com/checkcookie",
                    "https://acme.slack.com/ssb/redirect",
                ],
            ),
            (C1, vec![C1]),
        ],
    );
    let out = capture_team(&fake, &mut urls, "T0000001", Start::Navigate).await;
    assert!(matches!(out, TeamOutcome::Captured(_)), "{out:?}");
    assert_eq!(
        *fake.loads.borrow(),
        vec![LINK1.to_string(), C1.to_string()]
    );
}

#[tokio::test]
async fn login_link_straight_to_the_client() {
    let fake = Fake::new(Some(CONFIG2), vec![dcookie()]);
    fake.links
        .borrow_mut()
        .insert("T0000001".into(), LINK1.into());
    let mut urls = scripted(&fake, vec![(LINK1, vec![C1])]);
    let out = capture_team(&fake, &mut urls, "T0000001", Start::Navigate).await;
    assert!(matches!(out, TeamOutcome::Captured(_)), "{out:?}");
    assert_eq!(*fake.loads.borrow(), vec![LINK1.to_string()]);
}

#[tokio::test]
async fn login_link_back_to_sign_in_needs_the_user() {
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    fake.links
        .borrow_mut()
        .insert("T0000001".into(), LINK1.into());
    let mut urls = scripted(
        &fake,
        vec![(LINK1, vec!["https://app.slack.com/signin#/signin"])],
    );
    let out = capture_team(&fake, &mut urls, "T0000001", Start::Navigate).await;
    assert!(matches!(out, TeamOutcome::NeedsUser), "{out:?}");
}

#[tokio::test]
async fn without_a_link_the_client_is_loaded_and_a_handoff_still_needs_the_user() {
    let fake = Fake::new(Some(CONFIG), vec![dcookie()]);
    let mut urls = scripted(
        &fake,
        vec![(C1, vec!["https://acme.slack.com/ssb/redirect"])],
    );
    let out = capture_team(&fake, &mut urls, "T0000001", Start::Navigate).await;
    assert!(matches!(out, TeamOutcome::NeedsUser), "{out:?}");
    assert_eq!(*fake.loads.borrow(), vec![C1.to_string()]);
}
