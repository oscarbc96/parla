//! Dock badge through `NSDockTile`.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSWorkspace};
use objc2_foundation::NSString;

/// True when the main bundle has an identifier (a real `.app`).
pub fn is_bundled() -> bool {
    objc2_foundation::NSBundle::mainBundle()
        .bundleIdentifier()
        .is_some()
}

pub fn set_badge(count: Option<u32>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let label = count.map(|n| NSString::from_str(&n.to_string()));
    NSApplication::sharedApplication(mtm)
        .dockTile()
        .setBadgeLabel(label.as_deref());
}

/// The "Reduce motion" accessibility switch.
pub fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// The "Increase contrast" accessibility switch.
pub fn increase_contrast() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldIncreaseContrast()
}

/// Observes `NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification` on
/// the workspace notification centre for the life of the process. The centre
/// keeps the observer and its block, so the returned token is not stored.
pub fn on_accessibility_display_change(changed: impl Fn() + Send + Sync + 'static) {
    use objc2_app_kit::NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification;
    let block = block2::RcBlock::new(
        move |_: std::ptr::NonNull<objc2_foundation::NSNotification>| changed(),
    );
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: no sender filter (`obj` is nil), the block is `Send + Sync` so
    // it may run on whichever queue posts the notification.
    let _token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
            None,
            None,
            &block,
        )
    };
}

/// The compiled-in chime (`assets/sounds/parla-chime.wav`).
const CHIME: &[u8] = include_bytes!("../../../../assets/sounds/parla-chime.wav");

/// Plays the chime through `NSSound`; the returned closure stops it. `None`
/// when the sound cannot be made.
pub fn play_ringtone(looped: bool) -> Option<Box<dyn FnOnce()>> {
    use objc2::AnyThread;
    use objc2::rc::Retained;
    use objc2_app_kit::NSSound;
    use objc2_foundation::NSData;
    let data = NSData::with_bytes(CHIME);
    let sound: Retained<NSSound> = NSSound::initWithData(NSSound::alloc(), &data)?;
    sound.setLoops(looped);
    if !sound.play() {
        return None;
    }
    Some(Box::new(move || {
        sound.stop();
    }))
}

/// Plays the macOS "Tink" system sound once. Does nothing when the system has
/// no such sound.
pub fn play_notification_sound() {
    use objc2_app_kit::NSSound;
    use objc2_foundation::ns_string;
    if let Some(sound) = NSSound::soundNamed(ns_string!("Tink")) {
        sound.play();
    }
}

/// Whether the app has a Dock icon and menu bar (`Regular`) or runs without
/// either (`Accessory`, headless mode).
pub fn set_dock_visible(visible: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let policy = if visible {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    };
    NSApplication::sharedApplication(mtm).setActivationPolicy(policy);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
}

/// Seconds since the last input event in the login session (any app).
pub fn system_idle() -> Option<std::time::Duration> {
    // kCGEventSourceStateCombinedSessionState, kCGAnyInputEventType
    const COMBINED_SESSION: i32 = 0;
    const ANY_INPUT: u32 = !0;
    // SAFETY: a pure query with plain value arguments.
    let secs = unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION, ANY_INPUT) };
    (secs.is_finite() && secs >= 0.0).then(|| std::time::Duration::from_secs_f64(secs))
}

/// Observes `NSWorkspaceWillSleepNotification` and
/// `NSWorkspaceDidWakeNotification` on the main queue for the life of the app.
pub fn watch_sleep_wake(tx: futures::channel::mpsc::UnboundedSender<bool>) {
    use block2::RcBlock;
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::{NSNotification, NSOperationQueue};
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let queue = NSOperationQueue::mainQueue();
    // SAFETY: the notification names are AppKit's static strings.
    let names = unsafe {
        [
            (NSWorkspaceWillSleepNotification, false),
            (NSWorkspaceDidWakeNotification, true),
        ]
    };
    for (name, woke) in names {
        let tx = tx.clone();
        let block = RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
            let _ = tx.unbounded_send(woke);
        });
        // SAFETY: the block lives as long as the observer, which the app
        // keeps for its whole life (leaked on purpose: one per process).
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(name),
                None,
                Some(&queue),
                &block,
            )
        };
        std::mem::forget(observer);
    }
}
