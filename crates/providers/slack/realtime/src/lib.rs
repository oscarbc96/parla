//! Session websocket of the Slack provider.
//!
//! `event` holds the typed frames, `url` builds the connect URL, `conn` owns the
//! connection (keepalive, reconnect). See
//! `docs/providers/slack/web-client/realtime.md`.

pub mod conn;
pub mod event;
pub mod huddle;
pub mod url;

/// In-process fake websocket server, shared with the provider crate's tests.
#[cfg(feature = "test-support")]
#[path = "../tests/support/fake_server.rs"]
pub mod fake_server;
