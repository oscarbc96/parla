use settings::WorkspaceKey;
use slack_api::{SessionCookie, XoxcToken};
use slack_auth::*;
use url::Url;

fn creds(id: &str) -> WorkspaceCredentials {
    WorkspaceCredentials {
        workspace: WorkspaceKey::new("slack", id),
        user_id: "U0000001".to_string(),
        url: Url::parse("https://example.slack.com/").unwrap(),
        token: XoxcToken::parse("xoxc-0000000001-0000000002-0000000003-aaaa").unwrap(),
        cookie: SessionCookie::parse("d=xoxd-synthetic%2Fcookie; d-s=1700000000").unwrap(),
    }
}

#[tokio::test]
async fn memory_store_round_trip() {
    let store = MemoryStore::default();
    let c = creds("T0000001");
    store.save(&c).await.unwrap();
    let got = store.load(&c.workspace).await.unwrap().unwrap();
    assert_eq!(got.workspace, c.workspace);
    assert_eq!(got.user_id, c.user_id);
    assert_eq!(got.url, c.url);
    assert_eq!(got.token.expose(), c.token.expose());
    assert_eq!(got.cookie.header_value(), c.cookie.header_value());
    store.delete(&c.workspace).await.unwrap();
    assert!(store.load(&c.workspace).await.unwrap().is_none());
}

#[tokio::test]
async fn memory_store_is_per_workspace() {
    let store = MemoryStore::default();
    store.save(&creds("T0000001")).await.unwrap();
    assert!(
        store
            .load(&WorkspaceKey::new("slack", "T0000002"))
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn entry_url_is_per_team() {
    assert_eq!(
        entry_url(&WorkspaceKey::new("slack", "T0000001")),
        "parla://workspace/T0000001"
    );
}

#[test]
fn codec_round_trips_and_rejects_garbage() {
    let c = creds("T0000001");
    let bytes = encode_credentials(&c);
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains("\"v\":1"));
    let back = decode_credentials(&c.workspace, &c.user_id, &bytes).unwrap();
    assert_eq!(back.token.expose(), c.token.expose());
    assert!(decode_credentials(&c.workspace, "U1", b"not json").is_err());
    assert!(decode_credentials(&c.workspace, "U1", br#"{"v":2}"#).is_err());
}

#[test]
fn stored_url_off_slack_is_rejected() {
    for bad in [
        "http://example.slack.com/",
        "https://evil.example/",
        "https://slack.com.evil.example/",
        "https://user:pw@example.slack.com/",
        "https://example.slack.com:8443/",
    ] {
        let mut c = creds("T0000001");
        c.url = Url::parse(bad).unwrap();
        let bytes = encode_credentials(&c);
        assert!(
            decode_credentials(&c.workspace, &c.user_id, &bytes).is_err(),
            "{bad}"
        );
    }
}
