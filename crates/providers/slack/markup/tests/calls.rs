use domain::{Block, CallCard, CallKind, UserId};
use serde_json::Value;
use serde_json::value::RawValue;
use slack_markup::{huddle_card, parse_blocks, parse_message};

fn fixture(name: &str) -> Value {
    let p = format!("{}/tests/fixtures/calls/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn raw(v: &Value) -> Box<RawValue> {
    RawValue::from_string(v.to_string()).unwrap()
}

fn msg(name: &str) -> domain::RichText {
    let m = fixture(name);
    let blocks = m.get("blocks").map(raw);
    let room = m.get("room").map(raw);
    parse_message(
        m["text"].as_str().unwrap_or(""),
        blocks.as_deref(),
        None,
        room.as_deref(),
    )
}

fn first_call(rt: &domain::RichText) -> &CallCard {
    match &rt.blocks[0] {
        Block::Call(c) => c,
        b => panic!("{b:?}"),
    }
}

#[test]
fn huddle_thread_message_has_card_first() {
    let rt = msg("huddle_thread_live.json");
    let c = first_call(&rt);
    assert_eq!(c.kind, CallKind::Huddle);
    assert_eq!(c.started, Some(1794526177));
    assert_eq!(c.ended, None);
    assert_eq!(
        c.join_url.as_deref(),
        Some("https://app.slack.com/huddle/T0000001/C0000005")
    );
    // The message's own rich text follows the card.
    assert!(rt.plain_text().starts_with("Huddle"));
    assert!(rt.blocks.len() >= 2);
}

#[test]
fn ended_huddle_card_has_no_join() {
    let rt = msg("huddle_thread_ended.json");
    let c = first_call(&rt);
    assert_eq!(c.join_url, None);
    assert_eq!(c.ended, Some(1794527777));
    assert_eq!(
        &*c.participants,
        &[UserId("U0000001".into()), UserId("U0000002".into())]
    );
    assert_eq!(c.created_by, Some(UserId("U0000001".into())));
    // The message sender is the bot; the card names the room's creator.
    assert_eq!(huddle_card(&Value::Null), None);
}

#[test]
fn external_call_block_maps_provider_and_url() {
    let rt = msg("call_block_message.json");
    let c = first_call(&rt);
    assert_eq!(
        c.kind,
        CallKind::External {
            provider: "Example Meet".into()
        }
    );
    assert_eq!(c.title.as_deref(), Some("Weekly planning"));
    assert_eq!(
        c.join_url.as_deref(),
        Some("https://meet.example.com/join/123-456-789")
    );
    assert_eq!(c.started, Some(1794530000));
    assert_eq!(c.participants.len(), 3);
}

#[test]
fn call_block_without_url_still_renders() {
    let b = raw(&serde_json::json!([{"type":"call","call":{"v1":{"app_name":"Example Meet"}}}]));
    let blocks = parse_blocks(&b);
    let Block::Call(c) = &blocks[0] else {
        panic!("{:?}", blocks[0])
    };
    assert_eq!(c.join_url, None);
    assert!(matches!(&c.kind, CallKind::External { provider } if &**provider == "Example Meet"));
    let bare = raw(&serde_json::json!([{"type":"call"}]));
    assert!(matches!(&parse_blocks(&bare)[0], Block::Call(_)));
}

#[test]
fn plain_text_of_call_message() {
    assert_eq!(
        msg("call_block_message.json").plain_text(),
        "Call: Example Meet"
    );
    assert!(
        msg("huddle_thread_ended.json")
            .plain_text()
            .starts_with("Huddle")
    );
}
