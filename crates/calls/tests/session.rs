use std::sync::Arc;

use calls::{
    CallEffect, CallInput, CallKey, CallPhase, CallState, CaptureState, FailReason, HelperMsg,
    MediaKind, ParentMsg, step,
};
use domain::{CallSurface, ConversationId};
use settings::{CallSettings, WorkspaceKey};

fn key(c: &str) -> CallKey {
    CallKey {
        workspace: WorkspaceKey::new("fake", "w1"),
        conversation: ConversationId(Arc::from(c)),
    }
}
fn surface() -> CallSurface {
    CallSurface {
        url: Arc::from("https://app.example.test/h"),
        title: Arc::from("Huddle"),
        cookies: vec![],
        allowed_host_suffixes: Arc::from([Arc::from("example.test")]),
        signed_out_paths: Arc::from([Arc::from("/signin")]),
        user_agent: None,
    }
}
fn loaded() -> CallInput {
    CallInput::Helper(HelperMsg::Loaded {
        host: "app.example.test".into(),
        path: "/h".into(),
    })
}
fn cap(kind: MediaKind, state: CaptureState) -> CallInput {
    CallInput::Helper(HelperMsg::Capture { kind, state })
}

struct M {
    s: CallState,
    cfg: CallSettings,
}
impl M {
    fn new() -> Self {
        Self {
            s: CallState::idle(),
            cfg: CallSettings::default(),
        }
    }
    fn go(&mut self, i: CallInput) -> Vec<CallEffect> {
        step(&mut self.s, i, &self.cfg)
    }
    /// Idle -> Live on `c1`.
    fn live(c: &str) -> Self {
        let mut m = Self::new();
        m.go(CallInput::Open(key(c)));
        m.go(CallInput::SurfaceReady(surface()));
        m.go(CallInput::Helper(HelperMsg::Ready));
        m.go(loaded());
        assert_eq!(m.s.phase, CallPhase::Live);
        m
    }
}
fn has_kill(fx: &[CallEffect]) -> bool {
    fx.iter().any(|e| matches!(e, CallEffect::Kill))
}
fn fallbacks(fx: &[CallEffect]) -> Vec<FailReason> {
    fx.iter()
        .filter_map(|e| match e {
            CallEffect::Fallback { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect()
}
fn has_cancel(fx: &[CallEffect]) -> bool {
    fx.iter().any(|e| matches!(e, CallEffect::CancelLoadTimer))
}

#[test]
fn open_fetches_then_spawns_then_live() {
    let mut m = M::new();
    let fx = m.go(CallInput::Open(key("c1")));
    assert!(matches!(fx.as_slice(), [CallEffect::FetchSurface(k)] if *k == key("c1")));
    assert_eq!(m.s.phase, CallPhase::Fetching);
    let fx = m.go(CallInput::SurfaceReady(surface()));
    assert!(matches!(
        fx.as_slice(),
        [CallEffect::Spawn(_), CallEffect::StartLoadTimer]
    ));
    assert_eq!(m.s.phase, CallPhase::Starting);
    assert!(m.go(CallInput::Helper(HelperMsg::Ready)).is_empty());
    assert_eq!(m.s.phase, CallPhase::Loading);
    let fx = m.go(loaded());
    assert!(matches!(fx.as_slice(), [CallEffect::CancelLoadTimer]));
    assert_eq!(m.s.phase, CallPhase::Live);
    assert!(m.go(loaded()).is_empty());
}

#[test]
fn surface_failure_falls_back() {
    let mut m = M::new();
    m.go(CallInput::Open(key("c1")));
    let fx = m.go(CallInput::SurfaceFailed);
    assert_eq!(fallbacks(&fx), vec![FailReason::Surface]);
    assert!(!has_kill(&fx));
    assert_eq!(m.s.phase, CallPhase::Idle);

    let mut m = M::new();
    m.go(CallInput::Open(key("c1")));
    m.go(CallInput::SurfaceReady(surface()));
    let fx = m.go(CallInput::HelperStartFailed);
    assert_eq!(fallbacks(&fx), vec![FailReason::HelperStart]);
    assert!(has_cancel(&fx));
    assert_eq!(m.s.phase, CallPhase::Idle);
}

#[test]
fn signed_out_falls_back_once() {
    let mut m = M::new();
    m.go(CallInput::Open(key("c1")));
    m.go(CallInput::SurfaceReady(surface()));
    m.go(CallInput::Helper(HelperMsg::Ready));
    let mut all = m.go(CallInput::Helper(HelperMsg::SignedOut));
    all.extend(m.go(CallInput::Helper(HelperMsg::SignedOut)));
    assert_eq!(fallbacks(&all), vec![FailReason::SignedOut]);
    assert_eq!(
        all.iter().filter(|e| matches!(e, CallEffect::Kill)).count(),
        1
    );
    assert_eq!(m.s.phase, CallPhase::Idle);
}

#[test]
fn crash_before_live_falls_back_crash_after_live_ends() {
    let mut m = M::new();
    m.go(CallInput::Open(key("c1")));
    m.go(CallInput::SurfaceReady(surface()));
    let fx = m.go(CallInput::HelperExited);
    assert_eq!(fallbacks(&fx), vec![FailReason::Crashed]);
    assert!(!fx.iter().any(|e| matches!(e, CallEffect::LeaveProvider(_))));
    assert_eq!(m.s.phase, CallPhase::Idle);

    let mut m = M::live("c1");
    let fx = m.go(CallInput::HelperExited);
    assert!(fallbacks(&fx).is_empty());
    assert!(fx.iter().any(|e| matches!(e, CallEffect::LeaveProvider(_))));
}

#[test]
fn crash_ends_call_with_toast() {
    let mut m = M::live("c1");
    let fx = m.go(CallInput::HelperExited);
    assert!(fx.iter().any(|e| matches!(e, CallEffect::Toast(_))));
    assert!(
        fx.iter()
            .any(|e| matches!(e, CallEffect::LeaveProvider(k) if *k == key("c1")))
    );
    assert_eq!(m.s.phase, CallPhase::Idle);
    assert_eq!(m.s.key, None);
}

#[test]
fn closing_window_ends_call() {
    let mut m = M::live("c1");
    let fx = m.go(CallInput::Helper(HelperMsg::WindowClosed));
    assert!(fx.iter().any(|e| matches!(e, CallEffect::LeaveProvider(_))));
    assert!(!fx.iter().any(|e| matches!(e, CallEffect::Toast(_))));
    assert_eq!(m.s.phase, CallPhase::Idle);
    // The helper's exit afterwards is not a second leave.
    assert!(m.go(CallInput::HelperExited).is_empty());
}

#[test]
fn leave_sends_leave_then_idle_on_exit() {
    let mut m = M::live("c1");
    let fx = m.go(CallInput::Leave);
    assert!(matches!(
        fx.as_slice(),
        [CallEffect::Send(ParentMsg::Leave)]
    ));
    assert_eq!(m.s.phase, CallPhase::Leaving);
    assert!(m.go(CallInput::Leave).is_empty());
    let fx = m.go(CallInput::HelperExited);
    assert!(fx.iter().any(|e| matches!(e, CallEffect::LeaveProvider(_))));
    assert!(!fx.iter().any(|e| matches!(e, CallEffect::Toast(_))));
    assert_eq!(m.s.phase, CallPhase::Idle);
}

#[test]
fn second_join_asks_first() {
    let mut m = M::live("c1");
    let fx = m.go(CallInput::Open(key("c2")));
    assert!(matches!(fx.as_slice(), [CallEffect::AskSwitch { from, to }]
        if *from == key("c1") && *to == key("c2")));
    // "Stay" changes nothing: the call goes on.
    assert_eq!(m.s.phase, CallPhase::Live);
    assert_eq!(m.s.key, Some(key("c1")));
}

#[test]
fn same_conversation_join_shows_window() {
    let mut m = M::live("c1");
    let fx = m.go(CallInput::Open(key("c1")));
    assert!(matches!(fx.as_slice(), [CallEffect::Send(ParentMsg::Show)]));
    assert_eq!(m.s.phase, CallPhase::Live);
}

#[test]
fn mute_on_join_applies_once() {
    let mut m = M::live("c1");
    let fx = m.go(cap(MediaKind::Microphone, CaptureState::Active));
    assert!(matches!(
        fx.as_slice(),
        [CallEffect::Send(ParentMsg::SetMuted {
            kind: MediaKind::Microphone,
            muted: true
        })]
    ));
    m.go(cap(MediaKind::Microphone, CaptureState::Muted));
    // The user unmutes in the window; Active again is not a new join.
    assert!(
        m.go(cap(MediaKind::Microphone, CaptureState::Active))
            .is_empty()
    );

    let mut m = M::live("c1");
    m.cfg.mute_on_join = false;
    assert!(
        m.go(cap(MediaKind::Microphone, CaptureState::Active))
            .is_empty()
    );
}

#[test]
fn camera_left_alone_without_flag() {
    let mut m = M::live("c1");
    assert!(
        m.go(cap(MediaKind::Camera, CaptureState::Active))
            .is_empty()
    );
    assert_eq!(m.s.camera, CaptureState::Active);
}

#[test]
fn toggle_mute_hidden_when_unsupported() {
    let mut m = M::live("c1");
    m.go(cap(MediaKind::Microphone, CaptureState::Unsupported));
    assert!(m.go(CallInput::ToggleMute).is_empty());
    m.s = M::live("c1").s;
    m.go(cap(MediaKind::Microphone, CaptureState::Muted));
    let fx = m.go(CallInput::ToggleMute);
    assert!(matches!(
        fx.as_slice(),
        [CallEffect::Send(ParentMsg::SetMuted {
            kind: MediaKind::Microphone,
            muted: false
        })]
    ));
    m.go(cap(MediaKind::Microphone, CaptureState::Active));
    let fx = m.go(CallInput::ToggleMute);
    assert!(matches!(
        fx.as_slice(),
        [CallEffect::Send(ParentMsg::SetMuted {
            kind: MediaKind::Microphone,
            muted: true
        })]
    ));
}

#[test]
fn slow_load_marks_but_keeps_loading() {
    let mut m = M::new();
    m.go(CallInput::Open(key("c1")));
    m.go(CallInput::SurfaceReady(surface()));
    m.go(CallInput::Helper(HelperMsg::Ready));
    assert!(m.go(CallInput::LoadTimeout).is_empty());
    assert!(m.s.load_slow);
    assert_eq!(m.s.phase, CallPhase::Loading);
    // The timer fired, so going idle later has nothing to cancel.
    let fx = m.go(CallInput::Leave);
    assert!(matches!(
        fx.as_slice(),
        [CallEffect::Send(ParentMsg::Leave)]
    ));
    assert!(!has_cancel(&m.go(CallInput::HelperExited)));
    // A late load still goes live and clears the flag.
    let mut m = M::new();
    m.go(CallInput::Open(key("c1")));
    m.go(CallInput::SurfaceReady(surface()));
    m.go(CallInput::LoadTimeout);
    m.go(loaded());
    assert_eq!(m.s.phase, CallPhase::Live);
    assert!(!m.s.load_slow);
}

#[test]
fn provider_end_closes_window() {
    let mut m = M::live("c1");
    let fx = m.go(CallInput::HuddleEnded(key("other")));
    assert!(fx.is_empty());
    let fx = m.go(CallInput::HuddleEnded(key("c1")));
    assert!(matches!(
        fx.as_slice(),
        [CallEffect::Send(ParentMsg::Leave)]
    ));
    // The provider already knows; no leave call back to it.
    let fx = m.go(CallInput::HelperExited);
    assert!(!fx.iter().any(|e| matches!(e, CallEffect::LeaveProvider(_))));
    assert_eq!(m.s.phase, CallPhase::Idle);
}

#[test]
fn every_idle_path_cancels_timer() {
    type Go = fn(&mut M) -> Vec<CallEffect>;
    let paths: [(&str, Go); 6] = [
        ("start failed", |m| m.go(CallInput::HelperStartFailed)),
        ("exited", |m| m.go(CallInput::HelperExited)),
        ("signed out", |m| {
            m.go(CallInput::Helper(HelperMsg::SignedOut))
        }),
        ("window closed", |m| {
            m.go(CallInput::Helper(HelperMsg::WindowClosed))
        }),
        ("error", |m| {
            m.go(CallInput::Helper(HelperMsg::Error("x".into())))
        }),
        ("live", |m| m.go(loaded())),
    ];
    for (name, f) in paths {
        let mut m = M::new();
        m.go(CallInput::Open(key("c1")));
        m.go(CallInput::SurfaceReady(surface()));
        let fx = f(&mut m);
        assert!(has_cancel(&fx), "{name}");
    }
    // Cancelling while still fetching: no timer was armed, nothing to cancel.
    let mut m = M::new();
    m.go(CallInput::Open(key("c1")));
    assert!(!has_cancel(&m.go(CallInput::Leave)));
    assert_eq!(m.s.phase, CallPhase::Idle);
}
