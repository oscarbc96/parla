//! `SlackCodec` wire format and `SlackParser` behaviour.

use domain::{ContentParser, ExtCodec, ExtKind, ProviderExt};
use slack::{SlackCodec, SlackMessageExt, SlackParser, map_message};

fn msg(json: &str) -> domain::Message {
    map_message("C1", serde_json::from_str(json).unwrap())
}

#[test]
fn codec_round_trips_ext_with_version_byte() {
    let m = msg(
        r#"{"type":"message","subtype":"bot_message","bot_id":"B1","ts":"1.000001","text":"x"}"#,
    );
    let bytes = SlackCodec.encode(ExtKind::Message, &m.ext).unwrap();
    assert_eq!(bytes[0], 1);
    let back = SlackCodec.decode(ExtKind::Message, &bytes);
    assert_eq!(
        back.get::<SlackMessageExt>().unwrap().bot_id.as_deref(),
        Some("B1")
    );
}

#[test]
fn codec_unknown_version_is_none() {
    let mut bytes = br#"{"bot_id":"B1"}"#.to_vec();
    bytes.insert(0, 2);
    let d = SlackCodec.decode(ExtKind::Message, &bytes);
    assert!(d.get::<SlackMessageExt>().is_none());
    assert!(
        SlackCodec
            .decode(ExtKind::Message, &[])
            .get::<SlackMessageExt>()
            .is_none()
    );
}

#[test]
fn content_parser_prefers_rich_text_blocks() {
    let m = msg(r#"{"type":"message","ts":"1.000001","text":"fallback text",
        "blocks":[{"type":"rich_text","elements":[{"type":"rich_text_section","elements":[{"type":"text","text":"from blocks","style":{"bold":true}}]}]}]}"#);
    let rich = SlackParser.parse(&m);
    assert_eq!(rich.plain_text(), "from blocks");
}

#[test]
fn content_parser_without_ext_parses_mrkdwn() {
    let mut m = msg(r#"{"type":"message","ts":"1.000001","text":"*hi*"}"#);
    m.ext = ProviderExt::none();
    assert_eq!(SlackParser.parse(&m).plain_text(), "hi");
}

#[test]
fn parse_text_reads_mrkdwn_topics() {
    let rt = SlackParser.parse_text("Docs <https://example.com/x|here> in <#C1|general>");
    let domain::Block::Paragraph(i) = &rt.blocks[0] else {
        panic!("paragraph")
    };
    assert!(
        i.iter()
            .any(|x| matches!(x, domain::Inline::Link { label: Some(l), .. } if &**l == "here"))
    );
    assert!(
        i.iter()
            .any(|x| matches!(x, domain::Inline::Conversation { .. }))
    );
}
