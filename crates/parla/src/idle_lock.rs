//! Lock store when idle: after that
//! long without input every workspace closes its encrypted store (lean mode,
//! so connections, counts and notifications keep running), the store keys
//! cached in memory are dropped, the media viewer and the MCP audit window
//! close, pending MCP write prompts are declined, and every other Parla window
//! (main, pop-outs, Settings) shows the lock screen. Workspaces that start
//! while locked wait for Unlock before they open a store.
//!
//! Unlock first reads every open workspace's store key again (keychain, or
//! the development credentials file in a debug build). A failure or a
//! cancelled prompt keeps the lock and shows an inline error; Unlock retries.
//! Only then do the stores reopen, so a key failure never marks a store
//! unreadable.
//!
//! No polling and no idle timer: while the setting is on, one timer waits
//! for the earliest moment the lock could be due. When it fires it asks how
//! long the session has been idle (the OS clock on macOS, else Parla's own
//! last input) and either locks or waits the rest. It does not lock while
//! workspaces are still starting or loading their stores. On macOS a system
//! sleep longer than the setting locks at wake. Off, or locked, arms nothing.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use components::controls::{ButtonKind, button, on_activate_app};
use components::{Icon, IconName};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Global, SharedString, Subscription, Task,
    Window, div, prelude::*,
};
use settings::SettingsStore;
use store_sqlite::{DbId, KeySource};
use themes::{ActiveTheme as _, metrics};

use crate::workspaces::Workspaces;

/// How long the session has been idle, when the platform can say.
pub type IdleSource = Rc<dyn Fn(&App) -> Option<Duration>>;

/// The real source: the OS idle clock where there is one.
pub fn system_source() -> IdleSource {
    Rc::new(|_| crate::platform::system_idle())
}

/// How long to wait before the lock is due, or `None` when it is due now.
pub fn remaining(idle: Duration, after: Duration) -> Option<Duration> {
    after.checked_sub(idle).filter(|d| !d.is_zero())
}

/// Whether the lock is due when the system wakes: the idle time when it went
/// to sleep plus the time asleep reaches the setting.
pub fn due_after_sleep(idle_at_sleep: Duration, slept: Duration, after: Duration) -> bool {
    idle_at_sleep.saturating_add(slept) >= after
}

/// The lock screen's text, built when the lock goes on, not per frame.
#[derive(Clone)]
struct Texts {
    title: SharedString,
    detail: SharedString,
    unlock: SharedString,
}

pub struct IdleLock {
    workspaces: Entity<Workspaces>,
    keys: Option<Arc<dyn KeySource>>,
    source: IdleSource,
    after: Option<Duration>,
    /// Parla's own last input, for platforms without an OS idle clock.
    last_input: Instant,
    locked: bool,
    /// Unlock is reading the keys.
    unlocking: bool,
    error: Option<SharedString>,
    texts: Option<Texts>,
    /// When the system went to sleep, and how idle it was then.
    sleep: Option<(jiff::Timestamp, Duration)>,
    timer: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

/// The lock, for the views that cover themselves while it is on.
pub struct IdleLockHandle(pub Entity<IdleLock>);

impl Global for IdleLockHandle {}

/// Creates the lock, sets [`IdleLockHandle`] and arms it when the setting is
/// on. `keys` is the store's key source, whose cache the lock forgets.
pub fn install(
    cx: &mut App,
    workspaces: Entity<Workspaces>,
    keys: Option<Arc<dyn KeySource>>,
    source: IdleSource,
) -> Entity<IdleLock> {
    let lock = cx.new(|cx: &mut Context<IdleLock>| {
        let settings =
            cx.observe_global::<SettingsStore>(|l: &mut IdleLock, cx| l.settings_changed(cx));
        let weak = cx.weak_entity();
        let input = cx.intercept_keystrokes(move |_, _, cx| {
            if let Some(l) = weak.upgrade() {
                l.update(cx, |l, cx| l.note_input(cx));
            }
        });
        let mut l = IdleLock {
            workspaces,
            keys,
            source,
            after: after_setting(cx),
            last_input: cx.background_executor().now(),
            locked: false,
            unlocking: false,
            error: None,
            texts: None,
            sleep: None,
            timer: None,
            _subs: vec![settings, input],
        };
        l.check(cx);
        l
    });
    cx.set_global(IdleLockHandle(lock.clone()));
    lock
}

/// Feeds the OS sleep and wake notifications (macOS) to the lock.
pub fn watch_power(lock: &Entity<IdleLock>, cx: &mut App) {
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<bool>();
    if !crate::platform::watch_sleep_wake(tx) {
        return;
    }
    let lock = lock.downgrade();
    cx.spawn(async move |cx| {
        use futures::StreamExt as _;
        while let Some(woke) = rx.next().await {
            if lock.update(cx, |l, cx| l.on_power(woke, cx)).is_err() {
                return;
            }
        }
    })
    .detach();
}

fn after_setting(cx: &App) -> Option<Duration> {
    let m = cx
        .try_global::<SettingsStore>()
        .map_or(0, |s| s.current().storage.lock_after_minutes);
    (m > 0).then(|| Duration::from_secs(u64::from(m) * 60))
}

/// Whether the lock is on now (false when there is no lock).
pub fn is_locked(cx: &App) -> bool {
    cx.try_global::<IdleLockHandle>()
        .is_some_and(|h| h.0.read(cx).locked)
}

/// Records input in a Parla window (mouse down, scroll) on platforms without
/// an OS idle clock.
fn note_app_input(cx: &mut App) {
    if let Some(h) = cx.try_global::<IdleLockHandle>().map(|h| h.0.clone()) {
        h.update(cx, |l, cx| l.note_input(cx));
    }
}

/// Counts mouse down and scroll on `el` as input.
pub fn track_input<E: InteractiveElement>(el: E) -> E {
    el.capture_any_mouse_down(|_, _, cx| note_app_input(cx))
        .on_scroll(|_, _, cx| note_app_input(cx))
}

/// Mouse down and scroll in a Parla window count as input where the OS has no
/// idle clock; macOS counts every input itself, so there it adds nothing.
pub trait IdleInput: InteractiveElement + Sized {
    fn idle_input(self) -> Self {
        if cfg!(target_os = "macos") {
            self
        } else {
            track_input(self)
        }
    }
}

impl<E: InteractiveElement> IdleInput for E {}

impl IdleLock {
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Whether the one timer is pending (tests and the idle budget).
    pub fn timer_armed(&self) -> bool {
        self.timer.is_some()
    }

    /// Why the last Unlock failed, shown on the lock screen.
    pub fn unlock_error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    /// Records input seen by Parla. Cheap: no redraw and no new timer; the
    /// pending timer reads it when it fires.
    pub fn note_input(&mut self, cx: &mut Context<Self>) {
        self.last_input = cx.background_executor().now();
    }

    fn idle_for(&self, cx: &App) -> Duration {
        (self.source)(cx).unwrap_or_else(|| {
            cx.background_executor()
                .now()
                .saturating_duration_since(self.last_input)
        })
    }

    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let after = after_setting(cx);
        if after == self.after {
            return;
        }
        self.after = after;
        // The change came from the user, so it counts as input.
        self.note_input(cx);
        self.check(cx);
    }

    /// The system went to sleep (`false`) or woke (`true`).
    pub fn on_power(&mut self, woke: bool, cx: &mut Context<Self>) {
        if !woke {
            self.sleep = Some((components::now(cx), self.idle_for(cx)));
            return;
        }
        let Some((at, idle)) = self.sleep.take() else {
            self.check(cx);
            return;
        };
        let ms = components::now(cx).as_millisecond() - at.as_millisecond();
        let slept = Duration::from_millis(u64::try_from(ms).unwrap_or(0));
        match self.after {
            Some(after)
                if !self.locked
                    && due_after_sleep(idle, slept, after)
                    && self.workspaces.read(cx).ready_to_lock(cx) =>
            {
                self.lock(cx)
            }
            _ => self.check(cx),
        }
    }

    /// Locks when the session has been idle long enough, else waits the rest.
    /// While workspaces still start or load, it waits a full period again.
    fn check(&mut self, cx: &mut Context<Self>) {
        self.timer = None;
        let Some(after) = self.after else { return };
        if self.locked {
            return;
        }
        let wait = if self.workspaces.read(cx).ready_to_lock(cx) {
            remaining(self.idle_for(cx), after)
        } else {
            Some(after)
        };
        match wait {
            None => self.lock(cx),
            Some(wait) => {
                self.timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    this.update(cx, |l, cx| l.check(cx)).ok();
                }));
            }
        }
    }

    /// Closes every store, drops the cached keys, closes the windows that
    /// show workspace data, declines pending MCP prompts and covers the rest.
    pub fn lock(&mut self, cx: &mut Context<Self>) {
        if self.locked {
            return;
        }
        self.locked = true;
        self.timer = None;
        self.error = None;
        let l = i18n::gpui::localizer(cx);
        self.texts = Some(Texts {
            title: t!(l, "lock-title").into_owned().into(),
            detail: t!(l, "lock-detail").into_owned().into(),
            unlock: t!(l, "lock-unlock").into_owned().into(),
        });
        crate::log::info("idle lock: store locked");
        self.workspaces.update(cx, |w, cx| w.set_locked(true, cx));
        crate::media_viewer::close(cx);
        crate::code_viewer::close(cx);
        crate::mcp_status::close_audit_log(cx);
        crate::write_confirm_window::decline_all(cx);
        if let Some(keys) = self.keys.clone() {
            cx.background_executor()
                .spawn(async move { keys.forget_cached().await })
                .detach();
        }
        cx.notify();
    }

    /// Reads the store keys again, then reopens the active workspace's store
    /// and arms the timer. A failed read keeps the lock with an error.
    pub fn unlock(&mut self, cx: &mut Context<Self>) {
        if !self.locked || self.unlocking {
            return;
        }
        let Some(keys) = self.keys.clone() else {
            self.finish_unlock(cx);
            return;
        };
        self.unlocking = true;
        let ids = self.workspaces.read(cx).open_store_keys(cx);
        cx.spawn(async move |this, cx| {
            let mut failed = None;
            for ws in ids {
                if let Err(e) = keys.get_or_create(&DbId::Workspace(ws)).await {
                    failed = Some(e);
                    break;
                }
            }
            this.update(cx, |l, cx| {
                l.unlocking = false;
                match failed {
                    None => l.finish_unlock(cx),
                    Some(e) => {
                        crate::log::error(&format!("idle lock: cannot read a store key: {e}"));
                        let lz = i18n::gpui::localizer(cx);
                        l.error = Some(t!(lz, "lock-error").into_owned().into());
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn finish_unlock(&mut self, cx: &mut Context<Self>) {
        self.locked = false;
        self.error = None;
        self.texts = None;
        crate::log::info("idle lock: store unlocked");
        self.workspaces.update(cx, |w, cx| w.set_locked(false, cx));
        self.note_input(cx);
        self.check(cx);
        cx.notify();
    }
}

/// Unlocks from a click or key on the lock screen.
pub fn unlock(cx: &mut App) {
    if let Some(h) = cx.try_global::<IdleLockHandle>().map(|h| h.0.clone()) {
        h.update(cx, |l, cx| l.unlock(cx));
    }
}

/// The lock screen that covers a window while the store is locked, with
/// Unlock focused (`focus` is the covered view's own handle). Enter or Space
/// unlocks; nothing under it is drawn, so it takes no other keys.
pub fn cover(focus: &FocusHandle, window: &mut Window, cx: &mut App) -> gpui::Div {
    let colors = *cx.theme().colors();
    let danger = cx.theme().status().error;
    let (texts, error) = match cx.try_global::<IdleLockHandle>() {
        Some(h) => {
            let l = h.0.read(cx);
            (l.texts.clone(), l.error.clone())
        }
        None => (None, None),
    };
    if !focus.is_focused(window) {
        window.focus(focus, cx);
    }
    let texts = texts.unwrap_or_else(|| {
        let l = i18n::gpui::localizer(cx);
        Texts {
            title: t!(l, "lock-title").into_owned().into(),
            detail: t!(l, "lock-detail").into_owned().into(),
            unlock: t!(l, "lock-unlock").into_owned().into(),
        }
    });
    div()
        .size_full()
        .bg(colors.background)
        .child(crate::titlebar::column(lock_screen(
            &texts, error, focus, &colors, danger, cx,
        )))
}

fn lock_screen(
    texts: &Texts,
    error: Option<SharedString>,
    focus: &FocusHandle,
    colors: &themes::ThemeColors,
    danger: gpui::Hsla,
    cx: &App,
) -> gpui::Div {
    div()
        .debug_selector(|| "lock-screen".into())
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(metrics::SPACE_5)
        .p(metrics::SPACE_8)
        .bg(colors.background)
        .text_color(colors.text)
        .font_family(components::live_settings::ui_font(cx).0)
        .child(
            Icon::new(IconName::Lock)
                .size(metrics::SPACE_9)
                .color(colors.text_muted),
        )
        .child(
            div()
                .id("lock-title")
                .role(gpui::Role::Heading)
                .aria_level(1)
                .text_size(metrics::FONT_SIZE_2XL)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .child(texts.title.clone()),
        )
        .child(
            div()
                .max_w(metrics::LAYOUT_LOCK_TEXT)
                .text_center()
                .text_size(metrics::FONT_SIZE_SM)
                .text_color(colors.text_muted)
                .child(texts.detail.clone()),
        )
        .children(error.map(|e| {
            div()
                .debug_selector(|| "lock-error".into())
                .max_w(metrics::LAYOUT_LOCK_TEXT)
                .text_center()
                .text_size(metrics::FONT_SIZE_SM)
                .text_color(danger)
                .child(e)
        }))
        .child(on_activate_app(
            button(
                "lock-unlock",
                texts.unlock.clone(),
                ButtonKind::Primary,
                true,
                colors,
            )
            .track_focus(focus)
            .debug_selector(|| "lock-unlock".into()),
            |_, cx| unlock(cx),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_is_the_rest_of_the_wait_or_due() {
        let m = |n: u64| Duration::from_secs(n * 60);
        assert_eq!(remaining(m(0), m(15)), Some(m(15)));
        assert_eq!(remaining(m(10), m(15)), Some(m(5)));
        assert_eq!(remaining(m(15), m(15)), None);
        assert_eq!(remaining(m(20), m(15)), None);
    }
}
