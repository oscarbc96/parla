use domain::{ConversationId, MessageTs};
use std::sync::Arc;
use url::Url;

#[test]
fn parses_web_permalink() {
    let url = Url::parse("https://acme.slack.com/archives/C024BE7LH/p1700000000000100").unwrap();
    let p = slack::parse_permalink(&url).expect("parsed");
    assert_eq!(p.workspace_hint.as_ref(), "acme.slack.com");
    assert_eq!(p.conversation, ConversationId(Arc::from("C024BE7LH")));
    assert_eq!(p.ts, Some(MessageTs(Arc::from("1700000000.000100"))));
    assert_eq!(p.thread, None);
}

#[test]
fn parses_permalink_with_thread() {
    let url = Url::parse(
        "https://acme.slack.com/archives/C024BE7LH/p1700000000000100?thread_ts=1700000001.000200",
    )
    .unwrap();
    let p = slack::parse_permalink(&url).expect("parsed");
    assert_eq!(p.conversation, ConversationId(Arc::from("C024BE7LH")));
    assert_eq!(p.ts, Some(MessageTs(Arc::from("1700000000.000100"))));
    assert_eq!(p.thread, Some(MessageTs(Arc::from("1700000001.000200"))));
}

#[test]
fn rejects_other_hosts_and_shapes() {
    for bad in [
        "http://acme.slack.com/archives/C024BE7LH/p1700000000000100",
        "https://acme.slack.com/archives/C024BE7LH/1700000000.000100",
        "https://acme.slack.com/messages/C024BE7LH/p1700000000000100",
        "https://acme.slack.com/archives/C024BE7LH/p17000000000001",
        "https://acme.slack.com/archives//p1700000000000100",
    ] {
        let url = Url::parse(bad).unwrap();
        assert!(slack::parse_permalink(&url).is_none(), "{bad}");
    }
}
