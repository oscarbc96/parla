//! The parent/helper wire protocol: JSON lines over the helper's stdio.
//!
//! The session cookie travels only inside `ParentMsg::Open`, only through
//! `encode_parent`. `Debug` of every message redacts it, and helper messages
//! carry hosts and paths without query, never full URLs.

use std::sync::Arc;

use domain::{CallSurface, SecretText};
use serde::{Deserialize, Serialize};
use url::Url;

/// Longest accepted line, in bytes.
pub const MAX_LINE: usize = 64 * 1024;

/// State of a media capture device in the call window's webview.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    Unsupported,
    Inactive,
    Active,
    Muted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Microphone,
    Camera,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("line longer than {MAX_LINE} bytes")]
    TooLong,
    /// A well-formed message with a `type` this side does not know; callers
    /// ignore it and log the tag only.
    #[error("unknown message type `{0}`")]
    Unknown(String),
    #[error("malformed message")]
    Invalid,
}

/// A cookie on the wire; the value is secret (`Debug` redacted).
#[derive(Debug)]
pub struct WireCookie {
    pub name: Arc<str>,
    pub value: SecretText,
    pub domain: Arc<str>,
    pub path: Arc<str>,
    pub secure: bool,
    pub http_only: bool,
}

/// `domain::CallSurface` as sent to the helper.
#[derive(Debug)]
pub struct WireSurface {
    pub url: Arc<str>,
    pub title: Arc<str>,
    pub cookies: Vec<WireCookie>,
    pub allowed_host_suffixes: Vec<Arc<str>>,
    pub signed_out_paths: Vec<Arc<str>>,
    pub user_agent: Option<Arc<str>>,
}

impl From<CallSurface> for WireSurface {
    fn from(s: CallSurface) -> Self {
        // `SecretText` is not `Clone`, so move the surface apart.
        let CallSurface {
            url,
            title,
            cookies,
            allowed_host_suffixes,
            signed_out_paths,
            user_agent,
        } = s;
        Self {
            url,
            title,
            cookies: cookies
                .into_iter()
                .map(|c| WireCookie {
                    name: c.name,
                    value: c.value,
                    domain: c.domain,
                    path: c.path,
                    secure: c.secure,
                    http_only: c.http_only,
                })
                .collect(),
            allowed_host_suffixes: allowed_host_suffixes.to_vec(),
            signed_out_paths: signed_out_paths.to_vec(),
            user_agent,
        }
    }
}

/// Parent to helper.
#[derive(Debug)]
pub enum ParentMsg {
    Open(WireSurface),
    SetMuted { kind: MediaKind, muted: bool },
    Show,
    Leave,
}

/// Helper to parent. Never carries a query string or a cookie.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum HelperMsg {
    Ready,
    Loaded {
        host: String,
        path: String,
    },
    SignedOut,
    Capture {
        kind: MediaKind,
        state: CaptureState,
    },
    WindowClosed,
    Error(String),
}

impl HelperMsg {
    /// `Loaded` for `url`, without query or fragment.
    pub fn loaded(url: &Url) -> Self {
        let (host, path) = host_path(url);
        Self::Loaded { host, path }
    }
}

/// Host and path of `url`, dropping userinfo, port, query and fragment.
pub fn host_path(url: &Url) -> (String, String) {
    (
        url.host_str().unwrap_or_default().to_owned(),
        url.path().to_owned(),
    )
}

// ---- parent wire forms ---------------------------------------------------

#[derive(Serialize)]
struct CookieOut<'a> {
    name: &'a str,
    value: &'a str,
    domain: &'a str,
    path: &'a str,
    secure: bool,
    http_only: bool,
}

#[derive(Serialize)]
struct SurfaceOut<'a> {
    url: &'a str,
    title: &'a str,
    cookies: Vec<CookieOut<'a>>,
    allowed_host_suffixes: Vec<&'a str>,
    signed_out_paths: Vec<&'a str>,
    user_agent: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ParentOut<'a> {
    Open(SurfaceOut<'a>),
    SetMuted { kind: MediaKind, muted: bool },
    Show,
    Leave,
}

#[derive(Deserialize)]
struct CookieIn {
    name: String,
    value: String,
    domain: String,
    path: String,
    secure: bool,
    http_only: bool,
}

#[derive(Deserialize)]
struct SurfaceIn {
    url: String,
    title: String,
    cookies: Vec<CookieIn>,
    allowed_host_suffixes: Vec<String>,
    signed_out_paths: Vec<String>,
    user_agent: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ParentIn {
    Open(SurfaceIn),
    SetMuted { kind: MediaKind, muted: bool },
    Show,
    Leave,
}

const PARENT_TAGS: [&str; 4] = ["open", "set_muted", "show", "leave"];
const HELPER_TAGS: [&str; 6] = [
    "ready",
    "loaded",
    "signed_out",
    "capture",
    "window_closed",
    "error",
];

/// One JSON line (no trailing newline) for the helper's stdin.
pub fn encode_parent(m: &ParentMsg) -> String {
    let out = match m {
        ParentMsg::Open(s) => ParentOut::Open(SurfaceOut {
            url: &s.url,
            title: &s.title,
            cookies: s
                .cookies
                .iter()
                .map(|c| CookieOut {
                    name: &c.name,
                    value: c.value.expose(),
                    domain: &c.domain,
                    path: &c.path,
                    secure: c.secure,
                    http_only: c.http_only,
                })
                .collect(),
            allowed_host_suffixes: s.allowed_host_suffixes.iter().map(|x| &**x).collect(),
            signed_out_paths: s.signed_out_paths.iter().map(|x| &**x).collect(),
            user_agent: s.user_agent.as_deref(),
        }),
        ParentMsg::SetMuted { kind, muted } => ParentOut::SetMuted {
            kind: *kind,
            muted: *muted,
        },
        ParentMsg::Show => ParentOut::Show,
        ParentMsg::Leave => ParentOut::Leave,
    };
    serde_json::to_string(&out).expect("parent message serialises")
}

/// The `type` tag of a JSON object line, read from the text alone: a parent
/// line carries the session cookie, so it is never copied into a
/// `serde_json::Value` just to look at its tag.
fn peek_tag(line: &str) -> Option<&str> {
    let mut rest = line;
    while let Some(at) = rest.find("\"type\"") {
        rest = &rest[at + "\"type\"".len()..];
        let after = rest.trim_start();
        if let Some(after) = after.strip_prefix(':') {
            let after = after.trim_start().strip_prefix('"')?;
            return after.split_once('"').map(|(tag, _)| tag);
        }
    }
    None
}

fn check_line(line: &str, tags: &[&str]) -> Result<(), ProtocolError> {
    if line.len() > MAX_LINE {
        return Err(ProtocolError::TooLong);
    }
    match peek_tag(line) {
        Some(t) if tags.contains(&t) => Ok(()),
        Some(t) => Err(ProtocolError::Unknown(t.chars().take(32).collect())),
        None => Err(ProtocolError::Invalid),
    }
}

pub fn decode_parent(line: &str) -> Result<ParentMsg, ProtocolError> {
    check_line(line, &PARENT_TAGS)?;
    let m: ParentIn = serde_json::from_str(line).map_err(|_| ProtocolError::Invalid)?;
    Ok(match m {
        ParentIn::Open(s) => ParentMsg::Open(WireSurface {
            url: s.url.into(),
            title: s.title.into(),
            cookies: s
                .cookies
                .into_iter()
                .map(|c| WireCookie {
                    name: c.name.into(),
                    value: SecretText::new(c.value),
                    domain: c.domain.into(),
                    path: c.path.into(),
                    secure: c.secure,
                    http_only: c.http_only,
                })
                .collect(),
            allowed_host_suffixes: s
                .allowed_host_suffixes
                .into_iter()
                .map(Into::into)
                .collect(),
            signed_out_paths: s.signed_out_paths.into_iter().map(Into::into).collect(),
            user_agent: s.user_agent.map(Into::into),
        }),
        ParentIn::SetMuted { kind, muted } => ParentMsg::SetMuted { kind, muted },
        ParentIn::Show => ParentMsg::Show,
        ParentIn::Leave => ParentMsg::Leave,
    })
}

/// One JSON line (no trailing newline) for the helper's stdout.
pub fn encode_helper(m: &HelperMsg) -> String {
    serde_json::to_string(m).expect("helper message serialises")
}

pub fn decode_helper(line: &str) -> Result<HelperMsg, ProtocolError> {
    check_line(line, &HELPER_TAGS)?;
    serde_json::from_str(line).map_err(|_| ProtocolError::Invalid)
}
