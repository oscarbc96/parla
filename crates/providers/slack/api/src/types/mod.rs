//! Typed responses.

pub mod auth;
pub mod boot;
pub mod conversation;
pub mod file;
pub mod message;
pub mod reaction;
pub mod user;

pub use auth::AuthTest;
pub use boot::{Boot, BootSelf, BootTeam, ChannelCount, Counts};
pub use conversation::Conversation;
pub use file::{File, Thumb};
pub use message::{BotProfile, Edited, HistoryPage, Message};
pub use reaction::ReactionRaw;
pub use user::{Bot, BotIcons, Profile, User};
