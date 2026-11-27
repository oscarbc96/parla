//! The helper's decisions, free of GPUI and the webview: what to set, load,
//! allow, report and when to exit. `window.rs` wires it to the real window.

use std::fmt;
use std::sync::Arc;

use url::Url;

use crate::policy::{NavDecision, decide, is_signed_out, permit_media};
use crate::protocol::{CaptureState, HelperMsg, MediaKind, ParentMsg, WireCookie, WireSurface};

/// What the core needs from the webview.
pub trait CallWebview {
    fn set_cookies(&self, c: &[WireCookie]) -> Result<(), String>;
    fn load(&self, url: &str) -> Result<(), String>;
    /// `false` when the platform cannot mute.
    fn set_muted(&self, kind: MediaKind, muted: bool) -> bool;
}

/// What the shell must do after an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelperOut {
    Send(HelperMsg),
    OpenExternal(Url),
    Activate,
    Exit,
}

/// The host policy kept after `Open`; the cookies are gone by then.
struct Policy {
    suffixes: Vec<Arc<str>>,
    signed_out_paths: Vec<Arc<str>>,
}

pub struct HelperCore<W: CallWebview> {
    webview: W,
    policy: Option<Policy>,
    signed_out_sent: bool,
}

impl<W: CallWebview> fmt::Debug for HelperCore<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperCore")
            .field("open", &self.policy.is_some())
            .field("signed_out_sent", &self.signed_out_sent)
            .finish()
    }
}

fn error(msg: &str) -> HelperOut {
    HelperOut::Send(HelperMsg::Error(msg.to_owned()))
}

impl<W: CallWebview> HelperCore<W> {
    pub fn new(webview: W) -> Self {
        Self {
            webview,
            policy: None,
            signed_out_sent: false,
        }
    }

    pub fn on_parent(&mut self, m: ParentMsg) -> Vec<HelperOut> {
        match m {
            ParentMsg::Open(s) => self.open(s),
            ParentMsg::SetMuted { kind, muted } => {
                if self.webview.set_muted(kind, muted) {
                    // The capture observer reports the new state.
                    Vec::new()
                } else {
                    vec![HelperOut::Send(HelperMsg::Capture {
                        kind,
                        state: CaptureState::Unsupported,
                    })]
                }
            }
            ParentMsg::Show => vec![HelperOut::Activate],
            ParentMsg::Leave => vec![HelperOut::Exit],
        }
    }

    fn open(&mut self, s: WireSurface) -> Vec<HelperOut> {
        if self.policy.is_some() {
            return vec![error("already open")];
        }
        let allowed = Url::parse(&s.url)
            .map(|u| decide(&u, &s.allowed_host_suffixes) == NavDecision::Allow)
            .unwrap_or(false);
        if !allowed {
            return vec![error("surface url is not allowed"), HelperOut::Exit];
        }
        let WireSurface {
            url,
            cookies,
            allowed_host_suffixes,
            signed_out_paths,
            ..
        } = s;
        let set = self.webview.set_cookies(&cookies);
        // Zeroizes every cookie value (`SecretText` wipes on drop).
        drop(cookies);
        if set.is_err() {
            // The error text is ours; the platform message may echo input.
            return vec![error("could not set the session cookie"), HelperOut::Exit];
        }
        self.policy = Some(Policy {
            suffixes: allowed_host_suffixes,
            signed_out_paths,
        });
        if self.webview.load(&url).is_err() {
            return vec![error("could not load the surface"), HelperOut::Exit];
        }
        vec![HelperOut::Send(HelperMsg::Ready)]
    }

    fn signed_out(&mut self, url: &Url) -> bool {
        let Some(p) = &self.policy else { return false };
        is_signed_out(url, &p.suffixes, &p.signed_out_paths)
    }

    fn report_signed_out(&mut self) -> Vec<HelperOut> {
        if self.signed_out_sent {
            Vec::new()
        } else {
            self.signed_out_sent = true;
            vec![HelperOut::Send(HelperMsg::SignedOut)]
        }
    }

    /// `(allow, outputs)`; `allow` is the answer for the webview.
    pub fn on_navigation(&mut self, url: &Url) -> (bool, Vec<HelperOut>) {
        let Some(p) = &self.policy else {
            return (url.as_str() == "about:blank", Vec::new());
        };
        match decide(url, &p.suffixes) {
            NavDecision::Allow => {
                let out = if self.signed_out(url) {
                    self.report_signed_out()
                } else {
                    Vec::new()
                };
                (true, out)
            }
            NavDecision::OpenExternally => (false, vec![HelperOut::OpenExternal(url.clone())]),
            NavDecision::Block => (false, Vec::new()),
        }
    }

    pub fn on_page_loaded(&mut self, url: &Url) -> Vec<HelperOut> {
        let Some(p) = &self.policy else {
            return Vec::new();
        };
        if decide(url, &p.suffixes) != NavDecision::Allow || url.scheme() != "https" {
            return Vec::new();
        }
        if self.signed_out(url) {
            return self.report_signed_out();
        }
        vec![HelperOut::Send(HelperMsg::loaded(url))]
    }

    pub fn on_permission(&self, origin: &Url) -> bool {
        self.policy
            .as_ref()
            .is_some_and(|p| permit_media(origin, &p.suffixes))
    }

    pub fn on_capture(&mut self, kind: MediaKind, state: CaptureState) -> Vec<HelperOut> {
        vec![HelperOut::Send(HelperMsg::Capture { kind, state })]
    }

    pub fn on_window_closed(&mut self) -> Vec<HelperOut> {
        vec![HelperOut::Send(HelperMsg::WindowClosed), HelperOut::Exit]
    }

    pub fn on_stdin_eof(&mut self) -> Vec<HelperOut> {
        vec![HelperOut::Exit]
    }
}
