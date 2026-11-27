//! The call state machine: pure, no GPUI, no webview.
//!
//! `step` takes one input and returns the effects the controller executes.
//! Parla's state comes from helper events, never from the page.

use std::sync::Arc;

use domain::{CallSurface, ConversationId};
use settings::{CallSettings, WorkspaceKey};

use crate::protocol::{CaptureState, HelperMsg, MediaKind, ParentMsg};

/// Toast message ids the controller resolves through the app catalog.
pub const TOAST_HELPER_CRASHED: &str = "call-helper-crashed";

/// The conversation a call is about.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CallKey {
    pub workspace: WorkspaceKey,
    pub conversation: ConversationId,
}

/// Why the window could not join and the ladder moves on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailReason {
    Unsupported,
    Surface,
    HelperStart,
    SignedOut,
    Crashed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallPhase {
    Idle,
    Fetching,
    Starting,
    Loading,
    Live,
    Leaving,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallState {
    pub phase: CallPhase,
    pub key: Option<CallKey>,
    pub mic: CaptureState,
    pub camera: CaptureState,
    /// The load took long; the UI offers "Open in browser".
    pub load_slow: bool,
    mic_seen_active: bool,
    spawned: bool,
    timer_armed: bool,
    /// The provider ended the huddle; leaving needs no provider call.
    provider_ended: bool,
}

impl CallState {
    pub fn idle() -> Self {
        Self {
            phase: CallPhase::Idle,
            key: None,
            mic: CaptureState::Inactive,
            camera: CaptureState::Inactive,
            load_slow: false,
            mic_seen_active: false,
            spawned: false,
            timer_armed: false,
            provider_ended: false,
        }
    }
}

impl Default for CallState {
    fn default() -> Self {
        Self::idle()
    }
}

/// Inputs. A `SurfaceReady` or `SurfaceFailed` for a fetch the user already
/// cancelled is dropped by the controller; the machine cannot tell.
#[derive(Debug)]
pub enum CallInput {
    Open(CallKey),
    SurfaceReady(CallSurface),
    SurfaceFailed,
    Helper(HelperMsg),
    HelperStartFailed,
    HelperExited,
    LoadTimeout,
    Leave,
    ToggleMute,
    Show,
    HuddleEnded(CallKey),
}

#[derive(Debug)]
pub enum CallEffect {
    FetchSurface(CallKey),
    Spawn(CallSurface),
    Send(ParentMsg),
    StartLoadTimer,
    CancelLoadTimer,
    Kill,
    /// Move to the next ladder rung, once.
    Fallback {
        key: CallKey,
        reason: FailReason,
    },
    AskSwitch {
        from: CallKey,
        to: CallKey,
    },
    LeaveProvider(CallKey),
    /// A catalog message id for the app to show.
    Toast(Arc<str>),
}

fn to_idle(s: &mut CallState, fx: &mut Vec<CallEffect>) {
    if s.timer_armed {
        fx.push(CallEffect::CancelLoadTimer);
    }
    *s = CallState::idle();
}

fn begin(s: &mut CallState, key: CallKey, fx: &mut Vec<CallEffect>) {
    s.phase = CallPhase::Fetching;
    s.key = Some(key.clone());
    fx.push(CallEffect::FetchSurface(key));
}

/// Gives up the window for `reason`: kill, fall back once, go idle.
fn fall_back(s: &mut CallState, reason: FailReason, kill: bool, fx: &mut Vec<CallEffect>) {
    if kill && s.spawned {
        fx.push(CallEffect::Kill);
    }
    if let Some(key) = s.key.clone() {
        fx.push(CallEffect::Fallback { key, reason });
    }
    to_idle(s, fx);
}

fn leave(s: &mut CallState, fx: &mut Vec<CallEffect>) {
    match s.phase {
        CallPhase::Idle | CallPhase::Leaving => {}
        CallPhase::Fetching => to_idle(s, fx),
        CallPhase::Starting | CallPhase::Loading | CallPhase::Live => {
            s.phase = CallPhase::Leaving;
            fx.push(CallEffect::Send(ParentMsg::Leave));
        }
    }
}

/// The helper is gone or the window closed: the call is over.
fn ended(s: &mut CallState, fx: &mut Vec<CallEffect>) {
    if !s.provider_ended
        && let Some(key) = s.key.clone()
    {
        fx.push(CallEffect::LeaveProvider(key));
    }
    to_idle(s, fx);
}

pub fn step(s: &mut CallState, input: CallInput, settings: &CallSettings) -> Vec<CallEffect> {
    let mut fx = Vec::new();
    match input {
        CallInput::Open(key) => match (&s.phase, s.key.clone()) {
            (CallPhase::Idle, _) => begin(s, key, &mut fx),
            (_, Some(cur)) if cur == key => fx.push(CallEffect::Send(ParentMsg::Show)),
            (_, Some(from)) => {
                fx.push(CallEffect::AskSwitch { from, to: key });
            }
            (_, None) => {}
        },
        CallInput::SurfaceReady(surface) => {
            if s.phase == CallPhase::Fetching {
                s.phase = CallPhase::Starting;
                s.spawned = true;
                s.timer_armed = true;
                fx.push(CallEffect::Spawn(surface));
                fx.push(CallEffect::StartLoadTimer);
            }
        }
        CallInput::SurfaceFailed => {
            if s.phase == CallPhase::Fetching {
                fall_back(s, FailReason::Surface, false, &mut fx);
            }
        }
        CallInput::HelperStartFailed => {
            if s.phase == CallPhase::Starting {
                fall_back(s, FailReason::HelperStart, true, &mut fx);
            }
        }
        CallInput::LoadTimeout => {
            s.timer_armed = false;
            if matches!(s.phase, CallPhase::Starting | CallPhase::Loading) {
                s.load_slow = true;
            }
        }
        CallInput::Leave => leave(s, &mut fx),
        CallInput::HuddleEnded(key) => {
            if s.key.as_ref() == Some(&key) {
                s.provider_ended = true;
                leave(s, &mut fx);
            }
        }
        CallInput::Show => {
            if !matches!(s.phase, CallPhase::Idle | CallPhase::Fetching) {
                fx.push(CallEffect::Send(ParentMsg::Show));
            }
        }
        CallInput::ToggleMute => {
            if s.phase == CallPhase::Live
                && matches!(s.mic, CaptureState::Active | CaptureState::Muted)
            {
                fx.push(CallEffect::Send(ParentMsg::SetMuted {
                    kind: MediaKind::Microphone,
                    muted: s.mic != CaptureState::Muted,
                }));
            }
        }
        CallInput::HelperExited => match s.phase {
            CallPhase::Starting | CallPhase::Loading => {
                fall_back(s, FailReason::Crashed, false, &mut fx)
            }
            CallPhase::Live => {
                fx.push(CallEffect::Toast(TOAST_HELPER_CRASHED.into()));
                ended(s, &mut fx);
            }
            CallPhase::Leaving => ended(s, &mut fx),
            CallPhase::Idle | CallPhase::Fetching => {}
        },
        CallInput::Helper(msg) => helper(s, msg, settings, &mut fx),
    }
    fx
}

fn helper(s: &mut CallState, msg: HelperMsg, settings: &CallSettings, fx: &mut Vec<CallEffect>) {
    if matches!(s.phase, CallPhase::Idle | CallPhase::Fetching) {
        return;
    }
    match msg {
        HelperMsg::Ready => {
            if s.phase == CallPhase::Starting {
                s.phase = CallPhase::Loading;
            }
        }
        HelperMsg::Loaded { .. } => {
            if matches!(s.phase, CallPhase::Starting | CallPhase::Loading) {
                s.phase = CallPhase::Live;
                s.load_slow = false;
                if s.timer_armed {
                    s.timer_armed = false;
                    fx.push(CallEffect::CancelLoadTimer);
                }
            }
        }
        HelperMsg::SignedOut => {
            if s.phase != CallPhase::Leaving {
                fall_back(s, FailReason::SignedOut, true, fx);
            }
        }
        HelperMsg::Error(_) => {
            if matches!(s.phase, CallPhase::Starting | CallPhase::Loading) {
                fall_back(s, FailReason::Crashed, true, fx);
            }
        }
        HelperMsg::WindowClosed => match s.phase {
            CallPhase::Live | CallPhase::Leaving => {
                fx.push(CallEffect::Kill);
                ended(s, fx);
            }
            // Closed before the call began: nothing to leave, no fallback.
            _ => {
                fx.push(CallEffect::Kill);
                to_idle(s, fx);
            }
        },
        HelperMsg::Capture { kind, state } => match kind {
            MediaKind::Microphone => {
                s.mic = state;
                if state == CaptureState::Active && !s.mic_seen_active {
                    s.mic_seen_active = true;
                    if settings.mute_on_join {
                        fx.push(CallEffect::Send(ParentMsg::SetMuted { kind, muted: true }));
                    }
                }
            }
            MediaKind::Camera => {
                // The camera is left alone on join until the spike decides how the web client recovers.
                s.camera = state;
            }
        },
    }
}
