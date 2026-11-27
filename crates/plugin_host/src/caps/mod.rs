//! Capabilities: what the user granted a plugin and the check every host
//! call goes through.
//!
//! - [`Grant`]: what the user accepted at consent, built from the
//!   manifest and stored with the plugin's rows; editing files on disk
//!   never widens it.
//! - [`card_rows`]: the permission card as ids, risk and arguments; the
//!   words are Parla's own, from i18n.
//! - [`check`]: one host call against a grant and its context;
//!   [`check_result`]: one item of a result (search hit, hook event).
//! - Matchers: [`ArgShape::matches`](crate::manifest::ArgShape::matches)
//!   for `exec` (§7.4), [`UrlPattern`] for renderer links (§10.8) and
//!   [`NetworkHost::matches`](crate::manifest::NetworkHost::matches).
//! - [`area_capability`]: the provider feature each workspace area needs.

mod argv;
mod check;
mod grant;
mod map;
mod net;
mod risk;
mod url_pattern;

pub use check::{
    CallContext, ConversationRef, Denied, HostCall, HostCallKind, MessageTarget, Need, Origin,
    SearchKind, UserList, ViewId, check, check_result,
};
pub use grant::{Adds, ExecGrant, Grant, GrantDiff, Implied};
pub use map::{Area, area_capability, search_capability};
pub use risk::{CardRow, Risk, card_rows};
pub use url_pattern::UrlPattern;
