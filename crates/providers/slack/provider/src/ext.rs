//! Slack-only data carried on neutral types through `ProviderExt`. Only this
//! crate downcasts these; nothing outside `crates/providers/slack/` names them.

use std::sync::Arc;

use domain::{ContentParser, ExtCodec, ExtKind, Message, ProviderExt, RichText};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

/// Slack fields of a message that the neutral model has no slot for.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SlackMessageExt {
    pub subtype: Option<String>,
    pub bot_id: Option<String>,
    pub blocks: Option<Box<RawValue>>,
    pub files: Option<Box<RawValue>>,
    pub attachments: Option<Box<RawValue>>,
    pub reactions: Option<Box<RawValue>>,
    pub room: Option<Box<RawValue>>,
}

/// Slack fields of a conversation that the neutral model has no slot for.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SlackConversationExt {
    pub updated: Option<i64>,
}

/// Wire version written before the JSON body; other versions decode to none.
const CODEC_VERSION: u8 = 1;

fn versioned<T: Serialize>(v: &T) -> Option<Vec<u8>> {
    let mut out = vec![CODEC_VERSION];
    serde_json::to_writer(&mut out, v).ok()?;
    Some(out)
}

fn unversioned<T: serde::de::DeserializeOwned + Send + Sync + 'static>(
    bytes: &[u8],
) -> ProviderExt {
    match bytes.split_first() {
        Some((&CODEC_VERSION, body)) => {
            serde_json::from_slice::<T>(body).map_or_else(|_| ProviderExt::none(), ProviderExt::new)
        }
        _ => ProviderExt::none(),
    }
}

/// Persists the extension types above: one version byte, then JSON.
pub struct SlackCodec;

impl ExtCodec for SlackCodec {
    fn encode(&self, kind: ExtKind, ext: &ProviderExt) -> Option<Vec<u8>> {
        match kind {
            ExtKind::Message => versioned(ext.get::<SlackMessageExt>()?),
            ExtKind::Conversation => versioned(ext.get::<SlackConversationExt>()?),
            ExtKind::Member | ExtKind::File => None,
        }
    }

    fn decode(&self, kind: ExtKind, bytes: &[u8]) -> ProviderExt {
        match kind {
            ExtKind::Message => unversioned::<SlackMessageExt>(bytes),
            ExtKind::Conversation => unversioned::<SlackConversationExt>(bytes),
            ExtKind::Member | ExtKind::File => ProviderExt::none(),
        }
    }
}

/// Mrkdwn plus blocks and attachments into neutral rich text.
pub struct SlackParser;

impl ContentParser for SlackParser {
    fn parse(&self, m: &Message) -> RichText {
        match m.ext.get::<SlackMessageExt>() {
            Some(e) => slack_markup::parse_message(
                &m.text,
                e.blocks.as_deref(),
                e.attachments.as_deref(),
                e.room.as_deref(),
            ),
            None => slack_markup::parse_mrkdwn(&m.text),
        }
    }

    fn parse_text(&self, text: &str) -> RichText {
        slack_markup::parse_mrkdwn(text)
    }
}

pub(crate) fn codec() -> Arc<dyn ExtCodec> {
    Arc::new(SlackCodec)
}

pub(crate) fn parser() -> Arc<dyn ContentParser> {
    Arc::new(SlackParser)
}
