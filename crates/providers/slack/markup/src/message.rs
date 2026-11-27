//! Combine message text, blocks and attachments.

use domain::{Block, RichText};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::attachments::attachments_from_value;
use crate::blocks::blocks_from_value;
use crate::mrkdwn;

/// `text` is Slack's notification fallback: when the blocks render anything
/// supported, only the blocks show; otherwise `text` renders. Attachments
/// follow either way. A huddle `room` prepends its card.
pub fn parse_message(
    text: &str,
    blocks: Option<&RawValue>,
    attachments: Option<&RawValue>,
    room: Option<&RawValue>,
) -> RichText {
    let parsed = |r: &RawValue| serde_json::from_str::<Value>(r.get()).ok();
    let from_blocks = blocks
        .and_then(parsed)
        .map(|b| blocks_from_value(&b))
        .unwrap_or_default();
    let renders = from_blocks
        .iter()
        .any(|b| !matches!(b, Block::Unsupported { .. }));
    let card = room
        .and_then(parsed)
        .and_then(|r| crate::calls::huddle_card(&r));
    let mut out = if renders {
        from_blocks
    } else {
        let mut o = mrkdwn::parse(text).blocks.to_vec();
        o.extend(from_blocks);
        o
    };
    if let Some(c) = card {
        out.insert(0, Block::Call(c));
    }
    if let Some(a) = attachments.and_then(parsed) {
        out.extend(attachments_from_value(&a));
    }
    RichText {
        blocks: crate::tidy::blocks(out).into(),
    }
}
