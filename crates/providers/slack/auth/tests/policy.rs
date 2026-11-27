use slack_auth::webview::{
    Handoff, NavDecision, NewWindowAction, UrlKind, allow_navigation, classify_url, handoff,
    new_window_action, safari_user_agent, spa_route,
};
use url::Url;

fn u(s: &str) -> Url {
    Url::parse(s).unwrap()
}

#[test]
fn nav_policy() {
    assert_eq!(
        allow_navigation(&u("https://app.slack.com/x"), true),
        NavDecision::Allow
    );
    assert_eq!(
        allow_navigation(&u("https://slack.com/"), false),
        NavDecision::Allow
    );
    assert_eq!(
        allow_navigation(&u("https://evil.example/"), false),
        NavDecision::OpenExternally
    );
    assert_eq!(
        allow_navigation(&u("http://slack.com/"), true),
        NavDecision::Block
    );
    assert_eq!(
        allow_navigation(&u("https://idp.example/sso"), true),
        NavDecision::Allow
    );
    assert_eq!(
        allow_navigation(&u("https://notslack.com/"), true),
        NavDecision::Allow
    );
    assert_eq!(
        allow_navigation(&u("https://notslack.com/"), false),
        NavDecision::OpenExternally
    );
}

#[test]
fn user_agent_follows_os() {
    assert!(safari_user_agent(26).contains("Version/26.0 Safari/605.1.15"));
    assert!(safari_user_agent(15).contains("Version/18.0 Safari/605.1.15"));
    assert!(safari_user_agent(14).contains("Version/17.0 Safari/605.1.15"));
    assert!(safari_user_agent(26).starts_with("Mozilla/5.0 (Macintosh;"));
}

#[test]
fn handoff_with_team_id_in_the_url() {
    let client = u("https://app.slack.com/client/T0000001");
    assert_eq!(
        handoff(&u("slack://T0000001/magic-login/abc")),
        Some(Handoff::Client(client.clone()))
    );
    assert_eq!(
        handoff(&u("slack://channel?team=T0000001&id=C1")),
        Some(Handoff::Client(client))
    );
    assert_eq!(handoff(&u("slack://open")), None);
}

#[test]
fn handoff_redirect_only_names_the_workspace() {
    assert_eq!(
        handoff(&u("https://acme.slack.com/ssb/redirect")),
        Some(Handoff::Lookup {
            domain: "acme".into()
        })
    );
    assert!(handoff(&u("https://acme.slack.com/ssb/redirect?x=1")).is_some());
    assert_eq!(handoff(&u("https://app.slack.com/ssb/redirect")), None);
    assert_eq!(handoff(&u("https://slack.com/ssb/redirect")), None);
    assert_eq!(handoff(&u("https://acme.slack.com/client/T1")), None);
    assert_eq!(handoff(&u("https://evil.example/ssb/redirect")), None);
}

#[test]
fn handoff_is_intercepted_only_while_signing_in() {
    let r = u("https://acme.slack.com/ssb/redirect");
    assert_eq!(allow_navigation(&r, true), NavDecision::Handoff);
    assert_eq!(allow_navigation(&r, false), NavDecision::Allow);
    assert_eq!(
        allow_navigation(&u("slack://T0000001/x"), true),
        NavDecision::Handoff
    );
    assert_eq!(
        allow_navigation(&u("slack://T0000001/x"), false),
        NavDecision::Block
    );
    assert_eq!(classify_url(&r), UrlKind::DesktopHandoff);
}

#[test]
fn new_window_requests_never_open_a_window() {
    // Slack's workspace chooser opens "Open" in a new window.
    let open = |s: &str, signing_in| new_window_action(&u(s), signing_in);
    assert_eq!(
        open("https://acme.slack.com/ssb/redirect", true),
        NewWindowAction::Handoff
    );
    assert_eq!(
        open("slack://T0123ABC/join", true),
        NewWindowAction::Handoff
    );
    assert_eq!(
        open("https://app.slack.com/client/T0123ABC", true),
        NewWindowAction::LoadHere
    );
    assert_eq!(
        open("https://idp.example/sso", true),
        NewWindowAction::LoadHere
    );
    assert_eq!(
        open("https://evil.example/", false),
        NewWindowAction::OpenExternally
    );
    assert_eq!(open("http://slack.com/", true), NewWindowAction::Drop);
}

#[test]
fn spa_route_accepts_session_and_client_pages() {
    let from_bare = "https://slack.com/signin";
    let from_app = "https://app.slack.com/signin";
    assert_eq!(
        spa_route(from_bare, "https://slack.com/signin#/workspaces"),
        Some(u("https://slack.com/signin#/workspaces"))
    );
    assert_eq!(
        spa_route(from_app, "https://app.slack.com/signin#/workspaces"),
        Some(u("https://app.slack.com/signin#/workspaces"))
    );
    assert_eq!(
        spa_route(from_app, "https://app.slack.com/client/T0000001/C0000001"),
        Some(u("https://app.slack.com/client/T0000001/C0000001"))
    );
}

#[test]
fn spa_route_rejects_handoff_other_hosts_http_and_garbage() {
    let from = "https://slack.com/signin";
    assert_eq!(spa_route(from, "https://acme.slack.com/ssb/redirect"), None);
    assert_eq!(spa_route(from, "slack://T0000001/magic-login/abc"), None);
    assert_eq!(spa_route(from, "https://evil.example/signin"), None);
    assert_eq!(spa_route(from, "http://slack.com/signin"), None);
    assert_eq!(
        spa_route("http://app.slack.com/", "http://app.slack.com/client/T1"),
        None
    );
    assert_eq!(spa_route(from, "not a url"), None);
    assert_eq!(spa_route(from, ""), None);
}

#[test]
fn spa_route_rejects_messages_from_other_frames() {
    // Any frame can post to the handler: a third-party iframe, or a sign-in
    // provider page, must not be able to claim a Slack route.
    let client = "https://app.slack.com/client/T0000001";
    assert_eq!(spa_route("https://ads.example/frame", client), None);
    assert_eq!(
        spa_route("https://accounts.google.com/signin", client),
        None
    );
    assert_eq!(spa_route("https://acme.slack.com/sso", client), None);
    // A Slack frame on the other host cannot report a route on this one.
    assert_eq!(spa_route("https://slack.com/signin", client), None);
    assert_eq!(spa_route("not a url", client), None);
}
