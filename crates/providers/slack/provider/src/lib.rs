//! The Slack provider: implements `domain::ChatProvider` and
//! `domain::ProviderFactory` on top of `slack_api`, `slack_realtime`,
//! `slack_auth` and `slack_markup`. Slack types are mapped to neutral
//! `domain` types here and never leave this crate.

i18n::namespace!("slack");

mod channel_admin;
mod ext;
mod factory;
mod files;
mod huddles;
mod map;
mod permalink;
mod provider;
mod search;
pub mod sign_in;

pub use ext::{SlackCodec, SlackConversationExt, SlackMessageExt, SlackParser};
pub use factory::{CredentialBackend, CredentialSource, SlackFactory, factory, refresh_account};
pub use files::map_file;
pub use map::{
    group_dm_fallback_open, map_conversation, map_events_as, map_message, map_message_as, map_user,
};
pub use permalink::parse_permalink;
pub use provider::{ConnTune, SlackProvider, WsOptions};
pub use search::{snippet, to_query};
pub use sign_in::SlackSignIn;

/// The provider catalog (namespace `slack`): error texts and the unofficial notice.
pub fn catalog() -> &'static i18n::Catalog {
    &CATALOG
}
