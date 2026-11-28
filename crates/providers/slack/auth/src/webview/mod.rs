//! Login webview capture: a wry child webview in a GPUI window, a navigation
//! allowlist, and a state machine that reads the session after sign-in.

mod call;
mod capture;
#[cfg(target_os = "macos")]
mod mac;
mod persist;
mod policy;
mod sequence;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
mod ua;

pub use call::{CALL_HOST_SUFFIX, CALL_SIGNED_OUT_PATHS, huddle_url};
#[cfg(any(test, feature = "test-support"))]
pub use capture::run_capture;
pub use capture::{
    AdderWebview, CaptureFailure, CaptureState, NoLookup, RawCookie, TeamLookup, UrlKind,
    WebviewDriver, classify_url, cookie_from_jar, decode_eval, run_capture_with,
};
#[cfg(target_os = "macos")]
pub use mac::LoginWebview;
pub use persist::persist_verified;
pub use policy::{
    CHOOSER_HOOK_SCRIPT, CHOOSER_PREFIX, ChooserList, Handoff, NavDecision, NewWindowAction,
    ROUTE_HOOK_SCRIPT, allow_navigation, chooser_list, handoff, new_window_action, spa_route,
};
pub use sequence::{Start, TeamOutcome, TeamResult, capture_team, capture_teams};
pub use ua::os_major;
pub use ua::safari_user_agent;
