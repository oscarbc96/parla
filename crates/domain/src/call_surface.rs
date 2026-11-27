//! What a provider hands the call window: where to load, which cookies to
//! set from native code, which hosts may load and when we are signed out.

use std::sync::Arc;

use crate::SecretText;

/// One cookie set natively before the page loads. The value is secret.
#[derive(Debug)]
pub struct SurfaceCookie {
    pub name: Arc<str>,
    pub value: SecretText,
    pub domain: Arc<str>,
    pub path: Arc<str>,
    pub secure: bool,
    pub http_only: bool,
}

/// What a native (no webview) client needs to join a call's media session.
/// Neutral: a provider maps its own meeting and attendee fields onto it.
#[derive(Debug)]
pub struct NativeCallJoin {
    pub meeting_id: Arc<str>,
    pub attendee_id: Arc<str>,
    pub external_user_id: Arc<str>,
    /// Presented to the signalling server; secret.
    pub join_token: SecretText,
    pub signaling_url: Arc<str>,
    pub audio_host_url: Arc<str>,
    /// Absent when the provider does not give a TURN control endpoint.
    pub turn_control_url: Option<Arc<str>>,
}

/// The provider's call page and the rules the window enforces on it.
#[derive(Debug)]
pub struct CallSurface {
    pub url: Arc<str>,
    /// Window title, built by the provider from names it already holds.
    pub title: Arc<str>,
    pub cookies: Vec<SurfaceCookie>,
    /// A host equal to or ending in `.<suffix>` may load in the window.
    pub allowed_host_suffixes: Arc<[Arc<str>]>,
    /// Paths on an allowed host that mean the session is signed out.
    pub signed_out_paths: Arc<[Arc<str>]>,
    pub user_agent: Option<Arc<str>>,
}
