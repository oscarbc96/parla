//! Platform-specific pieces. Windows and Linux are no-ops until M7.

#[cfg(target_os = "macos")]
mod macos;

/// Whether this process runs from an app bundle that has an identifier. On
/// macOS the system notification centre needs one (`cargo run` has none, so
/// notifications cannot be posted); elsewhere always true.
pub fn is_bundled() -> bool {
    #[cfg(target_os = "macos")]
    return macos::is_bundled();
    #[cfg(not(target_os = "macos"))]
    true
}

/// What macOS says about Parla's notification permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationPermission {
    /// Authorized, provisional or ephemeral: notifications can be posted.
    Allowed,
    Denied,
    /// The system has never asked (or the bundle cannot ask).
    Undetermined,
}

impl NotificationPermission {
    /// The word used in the log line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Undetermined => "undetermined",
        }
    }
}

/// Asks for notification permission (the system prompt appears the first
/// time) and then reports the resulting setting through `tx`, on whichever
/// thread the system answers. Does nothing outside an app bundle: there the
/// UserNotifications framework aborts the process.
pub fn request_notification_permission(
    tx: futures::channel::mpsc::UnboundedSender<NotificationPermission>,
) {
    #[cfg(target_os = "macos")]
    macos::request_notification_permission(tx);
    #[cfg(not(target_os = "macos"))]
    let _ = tx;
}

/// Opens System Settings at Parla's notification pane (macOS only; a no-op
/// elsewhere).
pub fn open_notification_settings() {
    #[cfg(target_os = "macos")]
    macos::open_notification_settings();
}

/// Sets the dock badge (`None` clears it).
pub fn set_badge(count: Option<u32>) {
    #[cfg(target_os = "macos")]
    macos::set_badge(count);
    #[cfg(not(target_os = "macos"))]
    let _ = count;
}

/// Shows or hides the Dock icon (macOS only; headless mode hides it until a
/// window is shown).
pub fn set_dock_visible(visible: bool) {
    #[cfg(target_os = "macos")]
    macos::set_dock_visible(visible);
    #[cfg(not(target_os = "macos"))]
    let _ = visible;
}

/// Whether the Dock icon should show. It only goes away while the tray icon
/// is on: with `with_window` while the window is hidden, with `never` always
/// (menu bar only). Without the tray the icon stays, so the app can be reached.
pub fn dock_visible(tray_on: bool, setting: settings::DockIcon, window_visible: bool) -> bool {
    use settings::DockIcon;
    match setting {
        _ if !tray_on => true,
        DockIcon::Always => true,
        DockIcon::WithWindow => window_visible,
        DockIcon::Never => false,
    }
}

/// Whether the OS asks apps to reduce motion (macOS "Reduce motion"); `false`
/// where the platform has no such query yet.
pub fn reduce_motion() -> bool {
    #[cfg(target_os = "macos")]
    return macos::reduce_motion();
    #[cfg(not(target_os = "macos"))]
    false
}

/// Whether the OS asks apps for more contrast (macOS "Increase contrast");
/// `false` where the platform has no such query (there the high contrast
/// themes are picked by name).
pub fn increase_contrast() -> bool {
    #[cfg(target_os = "macos")]
    return macos::increase_contrast();
    #[cfg(not(target_os = "macos"))]
    false
}

/// Calls `changed` (on any thread) each time the OS accessibility display
/// options change (macOS posts one notification for contrast, motion,
/// transparency and colour switches). Event driven, no polling; a no-op where
/// the platform has no such notification.
pub fn on_accessibility_display_change(changed: impl Fn() + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    macos::on_accessibility_display_change(changed);
    #[cfg(not(target_os = "macos"))]
    let _ = changed;
}

/// Plays the ringtone (looping when `looped`); the returned closure stops it.
/// `None` where the platform has no player yet (Windows and Linux, M7).
pub fn play_ringtone(looped: bool) -> Option<Box<dyn FnOnce()>> {
    #[cfg(target_os = "macos")]
    return macos::play_ringtone(looped);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = looped;
        None
    }
}

/// Plays the system alert sound for a notification (macOS only; no-op
/// elsewhere until M7).
pub fn play_notification_sound() {
    #[cfg(target_os = "macos")]
    macos::play_notification_sound();
}

/// How long the whole session has gone without keyboard, mouse or trackpad
/// input (any app), for the idle lock. `None` where the platform has no
/// such query yet (Windows and Linux): the lock then counts from Parla's own
/// last key press.
pub fn system_idle() -> Option<std::time::Duration> {
    #[cfg(target_os = "macos")]
    return macos::system_idle();
    #[cfg(not(target_os = "macos"))]
    None
}

/// Sends `false` when the system goes to sleep and `true` when it wakes, for
/// the idle lock. Returns whether the platform reports them (macOS only).
pub fn watch_sleep_wake(tx: futures::channel::mpsc::UnboundedSender<bool>) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::watch_sleep_wake(tx);
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = tx;
        false
    }
}
