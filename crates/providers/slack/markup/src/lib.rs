//! Slack mrkdwn into the neutral `domain` rich-text model.

mod attachments;
mod blocks;
mod calls;
mod emoji;
mod encode;
mod entities;
mod message;
mod mrkdwn;
mod rich_text;
mod tidy;

use domain::{Mention, RichText};

pub use attachments::parse_attachments;
pub use blocks::parse_blocks;
pub use calls::huddle_card;
pub use emoji::emoji_ref;
pub use encode::SlackEncoder;
pub use message::parse_message;

/// Parse Slack mrkdwn into neutral rich text. Never panics.
pub fn parse_mrkdwn(text: &str) -> RichText {
    mrkdwn::parse(text)
}

/// Searchable plain text, using labels from tokens.
pub fn plain_text(text: &str) -> String {
    parse_mrkdwn(text).plain_text()
}

/// Mentions in order of first appearance, without duplicates.
pub fn mentions(text: &str) -> Vec<Mention> {
    mrkdwn::mentions(&parse_mrkdwn(text))
}
