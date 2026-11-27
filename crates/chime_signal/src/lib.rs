//! Signalling frames for a Chime-style meeting control websocket.
//!
//! The wire messages come from the vendored `SignalingProtocol.proto`
//! (see `NOTICE`). [`codec`] adds and strips the one-byte frame prefix and
//! [`machine`] is the sans-IO join, subscribe, mute and leave state machine.

pub mod codec;
pub mod machine;
pub mod secret;

/// Generated protobuf messages (proto2, no package).
#[allow(clippy::all, missing_docs)]
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

pub use codec::{Error, MAX_FRAME_LEN, decode, encode};
pub use machine::{Config, Event, Input, Machine, Output, Participant, State, TurnCredentials};
pub use proto::SdkSignalFrame;
pub use secret::Secret;
