use crate::ext::ProviderExt;
use crate::model::Message;
use crate::rich::RichText;

/// Lazy provider-markup to rich-text parsing.
pub trait ContentParser: Send + Sync {
    fn parse(&self, m: &Message) -> RichText;

    /// Parses free text in the provider's markup (a conversation topic, for
    /// instance); plain text by default.
    fn parse_text(&self, text: &str) -> RichText {
        RichText::plain(text)
    }
}

/// Treats message text as plain text.
pub struct PlainParser;

impl ContentParser for PlainParser {
    fn parse(&self, m: &Message) -> RichText {
        RichText::plain(&m.text)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtKind {
    Conversation,
    Message,
    Member,
    File,
}

/// Persists a provider's opaque extension data.
pub trait ExtCodec: Send + Sync {
    fn encode(&self, kind: ExtKind, ext: &ProviderExt) -> Option<Vec<u8>>;
    fn decode(&self, kind: ExtKind, bytes: &[u8]) -> ProviderExt;
}

pub struct NoExtCodec;

impl ExtCodec for NoExtCodec {
    fn encode(&self, _: ExtKind, _: &ProviderExt) -> Option<Vec<u8>> {
        None
    }
    fn decode(&self, _: ExtKind, _: &[u8]) -> ProviderExt {
        ProviderExt::none()
    }
}
