//! macOS: capture state through `WKWebView` (macOS 12+), with key-value
//! observation instead of polling. Nothing runs in the page.

use objc2::DefinedClass;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AnyThread, MainThreadMarker, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::{NSDictionary, NSKeyValueObservingOptions, NSString};
use objc2_web_kit::{WKMediaCaptureState, WKWebView};
use std::ffi::c_void;
use wry::WebViewExtMacOS;

use crate::protocol::{CaptureState, MediaKind};

pub const WINDOW_SUPPORTED: bool = true;

pub fn hide_from_dock() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm)
            .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
}

fn state_of(s: WKMediaCaptureState) -> CaptureState {
    if s == WKMediaCaptureState::Active {
        CaptureState::Active
    } else if s == WKMediaCaptureState::Muted {
        CaptureState::Muted
    } else {
        CaptureState::Inactive
    }
}

pub fn set_capture_muted(wv: &wry::WebView, kind: MediaKind, muted: bool) -> bool {
    let wk = wv.webview();
    let wk: &WKWebView = &wk;
    let sel = match kind {
        MediaKind::Microphone => sel!(setMicrophoneCaptureState:completionHandler:),
        MediaKind::Camera => sel!(setCameraCaptureState:completionHandler:),
    };
    // SAFETY: plain message sends to a live WKWebView on the main thread.
    unsafe {
        let responds: bool = msg_send![wk, respondsToSelector: sel];
        if !responds {
            return false;
        }
        let state = if muted {
            WKMediaCaptureState::Muted
        } else {
            WKMediaCaptureState::Active
        };
        match kind {
            MediaKind::Microphone => wk.setMicrophoneCaptureState_completionHandler(state, None),
            MediaKind::Camera => wk.setCameraCaptureState_completionHandler(state, None),
        }
    }
    true
}

struct Ivars {
    f: Box<dyn Fn(MediaKind, CaptureState)>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ParlaCaptureObserver"]
    #[ivars = Ivars]
    struct CaptureObserver;

    impl CaptureObserver {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSString, AnyObject>>,
            _context: *mut c_void,
        ) {
            let (Some(key), Some(obj)) = (key_path, object) else {
                return;
            };
            // SAFETY: we only observe a WKWebView, on the main thread.
            let wk: &WKWebView = unsafe { &*(obj as *const AnyObject as *const WKWebView) };
            let key = key.to_string();
            let (kind, raw) = match key.as_str() {
                "microphoneCaptureState" => {
                    (MediaKind::Microphone, unsafe { wk.microphoneCaptureState() })
                }
                "cameraCaptureState" => (MediaKind::Camera, unsafe { wk.cameraCaptureState() }),
                _ => return,
            };
            (self.ivars().f)(kind, state_of(raw));
        }
    }
);

pub fn observe_capture(wv: &wry::WebView, f: impl Fn(MediaKind, CaptureState) + 'static) {
    let wk = wv.webview();
    let this = CaptureObserver::alloc().set_ivars(Ivars { f: Box::new(f) });
    // SAFETY: `init` of NSObject; KVO registration on a live WKWebView.
    let observer: Retained<CaptureObserver> = unsafe { msg_send![super(this), init] };
    let opts = NSKeyValueObservingOptions::Initial | NSKeyValueObservingOptions::New;
    for key in ["microphoneCaptureState", "cameraCaptureState"] {
        let key = NSString::from_str(key);
        // SAFETY: the observer is leaked below, so it outlives the webview; the
        // process ends with the call.
        unsafe {
            let _: () = msg_send![
                &*wk,
                addObserver: &*observer,
                forKeyPath: &*key,
                options: opts,
                context: std::ptr::null_mut::<c_void>()
            ];
        }
    }
    std::mem::forget(observer);
}

pub fn open_external(url: &url::Url) {
    let _ = std::process::Command::new("open").arg(url.as_str()).spawn();
}
