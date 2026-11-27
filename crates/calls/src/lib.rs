//! Neutral calls: the call window's navigation policy, the parent/helper
//! wire protocol, the call state machine and the fallback ladder. Nothing
//! provider-specific may appear here; the provider hands in a
//! `domain::CallSurface` that carries every host and path.

pub mod ladder;
pub mod native;
pub mod policy;
pub mod protocol;
pub mod session;

pub use ladder::{JoinRoute, ladder};
pub use policy::{NavDecision, decide, host_allowed, is_signed_out, permit_media};
pub use protocol::{
    CaptureState, HelperMsg, MAX_LINE, MediaKind, ParentMsg, ProtocolError, WireCookie,
    WireSurface, decode_helper, decode_parent, encode_helper, encode_parent, host_path,
};
pub use session::{
    CallEffect, CallInput, CallKey, CallPhase, CallState, FailReason, TOAST_HELPER_CRASHED, step,
};

pub mod controller;
pub mod helper;
pub mod platform;
