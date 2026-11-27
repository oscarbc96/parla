//! The menu-bar (tray) icon: Parla's own template glyph, a dot and the mention
//! count when something needs you, and a menu with one entry per workspace.
//!
//! Event driven, like the dock badge: a change of any workspace (a conversation
//! list, a pause, an expired session, a workspace added or activated) asks for
//! a rebuild with [`Tray::request`]. Requests coalesce into one rebuild a frame
//! later through a one-shot timer, so a burst of messages rebuilds once and an
//! idle app has no timer at all. The real status item is created on
//! the main thread after GPUI starts; creating it earlier fails.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use components::NotifyControls as _;
use gpui::{App, Global, QuitMode, SharedString, Window};
use jiff::tz::TimeZone;
use settings::{SettingsStore, WorkspaceKey};

use crate::workspaces::WorkspacesHandle;

/// One frame at 60 Hz: how long requests are held to coalesce.
const FRAME: Duration = Duration::from_millis(16);

/// One line of the tray menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayEntry {
    /// Unread conversations and mentions over every workspace; a title, not a
    /// command.
    Header {
        unread: u32,
        mentions: u32,
    },
    /// `index` is the position in [`TrayView::keys`].
    Workspace {
        name: SharedString,
        mentions: u32,
        expired: bool,
        index: usize,
    },
    /// A submenu of [`TrayView::pause_items`].
    PauseNotifications,
    Resume,
    SetStatus,
    Settings,
    Open,
    Quit,
    Separator,
}

/// What a click on a menu entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Workspace(usize),
    /// Pause the active workspace's notifications for this many minutes.
    PauseMinutes(u32),
    /// Until tomorrow morning, counted from the click.
    PauseTomorrow,
    /// Opens the pause menu of the window, which has the duration field.
    PauseCustom,
    Resume,
    SetStatus,
    Settings,
    Open,
    Quit,
}

/// The menu-bar glyphs: black-on-transparent PNGs drawn at 2x. The status item
/// scales an image taller than 22 pt down to 22 pt, so they are 44 px canvases
/// with the bubble inside padding (about 16 pt), the weight of the system's own
/// items. They are set as template images (`with_icon_templated`: only the
/// alpha is drawn, tinted for light and dark menu bars). Regenerate with `mise run assets:tray`.
const GLYPH_PLAIN: &[u8] = include_bytes!("../../../assets/icons/tray/tray.png");
const GLYPH_DOT: &[u8] = include_bytes!("../../../assets/icons/tray/tray-dot.png");

/// The glyph PNG for `badge` unread mentions (a dot while there are some).
pub fn glyph_png(badge: u32) -> &'static [u8] {
    if badge > 0 { GLYPH_DOT } else { GLYPH_PLAIN }
}

/// The menu for `ws` (name, mentions, session expired) with `unread`
/// unread conversations over all of them, in order. Pure.
pub fn tray_menu_model(
    ws: &[(SharedString, u32, bool)],
    unread: u32,
    paused: bool,
) -> Vec<TrayEntry> {
    let mut out = Vec::new();
    if !ws.is_empty() {
        out.push(TrayEntry::Header {
            unread,
            mentions: ws.iter().map(|w| w.1).sum(),
        });
        out.push(TrayEntry::Separator);
    }
    out.extend(
        ws.iter()
            .enumerate()
            .map(|(index, (name, mentions, expired))| TrayEntry::Workspace {
                name: name.clone(),
                mentions: *mentions,
                expired: *expired,
                index,
            }),
    );
    if !ws.is_empty() {
        out.push(TrayEntry::Separator);
        out.push(if paused {
            TrayEntry::Resume
        } else {
            TrayEntry::PauseNotifications
        });
        out.push(TrayEntry::SetStatus);
        out.push(TrayEntry::Separator);
    }
    out.push(TrayEntry::Settings);
    out.push(TrayEntry::Open);
    out.push(TrayEntry::Quit);
    out
}

/// The menu item id of an entry, the inverse of [`action_for_id`].
pub fn id_of(entry: &TrayEntry) -> Option<String> {
    Some(match entry {
        TrayEntry::Workspace { index, .. } => format!("ws:{index}"),
        TrayEntry::Header { .. } => "header".into(),
        TrayEntry::PauseNotifications => "pause".into(),
        TrayEntry::Resume => "resume".into(),
        TrayEntry::SetStatus => "status".into(),
        TrayEntry::Settings => "settings".into(),
        TrayEntry::Open => "open".into(),
        TrayEntry::Quit => "quit".into(),
        TrayEntry::Separator => return None,
    })
}

pub fn action_for_id(id: &str) -> Option<TrayAction> {
    match id {
        "resume" => Some(TrayAction::Resume),
        "status" => Some(TrayAction::SetStatus),
        "settings" => Some(TrayAction::Settings),
        "pause:tomorrow" => Some(TrayAction::PauseTomorrow),
        "pause:custom" => Some(TrayAction::PauseCustom),
        "open" => Some(TrayAction::Open),
        "quit" => Some(TrayAction::Quit),
        _ => {
            if let Some(minutes) = id.strip_prefix("pause:") {
                return minutes.parse().ok().map(TrayAction::PauseMinutes);
            }
            id.strip_prefix("ws:")?
                .parse()
                .ok()
                .map(TrayAction::Workspace)
        }
    }
}

/// Everything the status item shows, already localized.
#[derive(Clone, Debug, PartialEq)]
pub struct TrayView {
    pub entries: Vec<TrayEntry>,
    /// Text of each entry, parallel to `entries` (empty for separators).
    pub labels: Vec<String>,
    /// The pause submenu: the menu item id and the text of each choice.
    pub pause_items: Vec<(String, String)>,
    /// Workspace of each `TrayEntry::Workspace::index`.
    pub keys: Vec<WorkspaceKey>,
    /// Mentions over all workspaces: the dot shows above 0, the title is it.
    pub badge: u32,
    pub tooltip: String,
}

/// Where the model goes: the real status item, or a recorder in tests.
pub trait TrayPort {
    fn show(&self, view: &TrayView, cx: &mut App);
    /// Takes the icon away (the setting was turned off).
    fn remove(&self);
    /// Hides the app (closing the window while the icon is on).
    fn hide_app(&self, cx: &mut App) {
        cx.hide();
    }
    /// Shows or hides the Dock icon (the `dock_icon` setting).
    fn set_dock(&self, visible: bool) {
        crate::platform::set_dock_visible(visible);
    }
    /// False when the status item could not be created: closing the window
    /// then closes it, so the app is never left without a way back.
    fn is_available(&self) -> bool {
        true
    }
}

struct Inner {
    port: Rc<dyn TrayPort>,
    dirty: Cell<bool>,
    rebuilds: Cell<usize>,
    last: RefCell<Option<TrayView>>,
    /// The app is hidden because the window was closed with the icon on.
    hidden: Cell<bool>,
    /// Dock icon state last applied. Starts `true`: a headless start hides the
    /// icon directly (`platform::set_dock_visible(false)`), and the `dock_pending`
    /// flag in `main.rs` restores it when the first window shows.
    dock: Cell<bool>,
}

/// The tray's global; a cheap handle.
#[derive(Clone)]
pub struct Tray(Rc<Inner>);

impl Global for Tray {}

/// GPUI's quit behaviour for the setting: with the icon the app keeps running
/// without windows; without it the last window closing quits.
pub fn quit_mode_for(tray: bool) -> QuitMode {
    if tray {
        QuitMode::Explicit
    } else {
        QuitMode::LastWindowClosed
    }
}

/// Whether the Dock icon shows with a window open, for the current settings
/// (a launch that hid it brings it back through this).
pub fn dock_for_window(cx: &App) -> bool {
    let setting = cx
        .try_global::<SettingsStore>()
        .map(|s| s.current().dock_icon)
        .unwrap_or_default();
    crate::platform::dock_visible(Tray::usable(cx), setting, true)
}

fn enabled(cx: &App) -> bool {
    cx.try_global::<SettingsStore>()
        .is_none_or(|s| s.current().tray)
}

impl Tray {
    pub fn new(port: Rc<dyn TrayPort>) -> Self {
        Self(Rc::new(Inner {
            port,
            dirty: Cell::new(false),
            rebuilds: Cell::new(0),
            last: RefCell::new(None),
            hidden: Cell::new(false),
            dock: Cell::new(true),
        }))
    }

    /// Makes this the app's tray, applies the quit mode and shows the icon.
    pub fn install(self, cx: &mut App) {
        cx.set_global(self);
        Self::settings_changed(cx);
    }

    /// Models built so far (tests check that bursts coalesce).
    pub fn rebuilds(&self) -> usize {
        self.0.rebuilds.get()
    }

    /// The `tray` setting may have changed: quit mode, icon on or off.
    pub fn settings_changed(cx: &mut App) {
        if cx.try_global::<Tray>().is_none() {
            return;
        }
        let usable = enabled(cx)
            && cx
                .try_global::<Tray>()
                .is_none_or(|t| t.0.port.is_available());
        cx.set_quit_mode(quit_mode_for(usable));
        Self::sync_dock(cx);
        Self::request(cx);
    }

    /// The window was hidden or is about to show: the Dock icon follows the
    /// `dock_icon` setting. Call it before activating, so the app regains
    /// focus after the activation policy switches back to a regular app.
    pub fn set_window_hidden(hidden: bool, cx: &mut App) {
        let Some(this) = cx.try_global::<Tray>().cloned() else {
            return;
        };
        this.0.hidden.set(hidden);
        Self::sync_dock(cx);
    }

    /// The setting is on and the status item is, as far as is known, there:
    /// the app can be reached through it. A status item that could not be
    /// created only shows when the first model is shown, so this turns false
    /// then (see `rebuild`), not before.
    fn usable(cx: &App) -> bool {
        cx.try_global::<Tray>()
            .is_some_and(|t| enabled(cx) && t.0.port.is_available())
    }

    fn sync_dock(cx: &mut App) {
        let Some(this) = cx.try_global::<Tray>().cloned() else {
            return;
        };
        let setting = cx
            .try_global::<SettingsStore>()
            .map(|s| s.current().dock_icon)
            .unwrap_or_default();
        let want = crate::platform::dock_visible(Self::usable(cx), setting, !this.0.hidden.get());
        if this.0.dock.replace(want) != want {
            this.0.port.set_dock(want);
        }
    }

    /// Something the menu shows may have changed. Cheap: the first call arms
    /// one frame-long timer, the others only see the flag.
    pub fn request(cx: &mut App) {
        let Some(this) = cx.try_global::<Tray>().cloned() else {
            return;
        };
        if this.0.dirty.replace(true) {
            return;
        }
        cx.spawn(async move |cx| {
            cx.background_executor().timer(FRAME).await;
            cx.update(|cx| this.rebuild(cx));
        })
        .detach();
    }

    fn rebuild(&self, cx: &mut App) {
        self.0.dirty.set(false);
        self.0.rebuilds.set(self.0.rebuilds.get() + 1);
        if !enabled(cx) {
            if self.0.last.take().is_some() {
                self.0.port.remove();
            }
            return;
        }
        let view = build_view(cx);
        if self.0.last.borrow().as_ref() == Some(&view) {
            return;
        }
        self.0.port.show(&view, cx);
        *self.0.last.borrow_mut() = Some(view);
        // The port may just have found it cannot show the icon: the Dock icon
        // and the quit mode follow, or `dock_icon: never` would leave the app
        // with no Dock icon and no tray.
        cx.set_quit_mode(quit_mode_for(Self::usable(cx)));
        Self::sync_dock(cx);
    }

    /// Closing the window hides the app while the icon can bring it back;
    /// otherwise the window closes as usual.
    pub fn guard_window(window: &mut Window, cx: &App) {
        window.on_window_should_close(cx, |_, cx| {
            if !Tray::hides_on_close(cx) {
                return true;
            }
            Tray::hide_for_close(cx);
            false
        });
    }

    /// Whether closing the main window hides the app (the icon is on and the
    /// status item exists) instead of closing it.
    pub fn hides_on_close(cx: &App) -> bool {
        cx.try_global::<Tray>()
            .is_some_and(|this| enabled(cx) && this.0.port.is_available())
    }

    /// Hides the app the way closing the main window does with the icon on.
    pub fn hide_for_close(cx: &mut App) {
        let Some(port) = cx.try_global::<Tray>().map(|t| t.0.port.clone()) else {
            return;
        };
        port.hide_app(cx);
        Tray::set_window_hidden(true, cx);
    }

    /// A click on a menu entry.
    pub fn dispatch(action: TrayAction, cx: &mut App) {
        let handle = cx.try_global::<WorkspacesHandle>().map(|h| h.0.clone());
        match action {
            TrayAction::Quit => cx.quit(),
            TrayAction::Open => show_window(cx),
            TrayAction::Workspace(i) => {
                let key = cx.try_global::<Tray>().and_then(|t| {
                    t.0.last
                        .borrow()
                        .as_ref()
                        .and_then(|v| v.keys.get(i).cloned())
                });
                if let (Some(h), Some(key)) = (handle, key) {
                    h.update(cx, |w, cx| w.activate(&key, cx));
                }
                show_window(cx);
            }
            TrayAction::PauseMinutes(_) | TrayAction::PauseTomorrow | TrayAction::Resume => {
                let session = handle.and_then(|h| h.read(cx).active_session());
                if let Some(s) = session {
                    // "Until tomorrow" is counted from the click, not from when
                    // the menu was built.
                    let minutes = match action {
                        TrayAction::PauseMinutes(m) => Some(m),
                        TrayAction::PauseTomorrow => {
                            let zone = s.read(cx).notify_config(cx).zone;
                            let now = components::now(cx).to_zoned_datetime(zone);
                            Some(components::minutes_until_tomorrow(&now))
                        }
                        _ => None,
                    };
                    s.update(cx, |s, cx| match minutes {
                        Some(m) => s.pause(m, cx),
                        None => s.resume(cx),
                    });
                }
            }
            TrayAction::PauseCustom => {
                show_window(cx);
                dispatch_in_window(cx, crate::actions::PauseNotifications);
            }
            TrayAction::SetStatus => {
                show_window(cx);
                dispatch_in_window(cx, crate::actions::SetStatus);
            }
            TrayAction::Settings => crate::app_state::open_settings(cx),
        }
    }
}

/// Runs `action` in the window in front once it is there (after a frame, so a
/// window the tray just created or showed is the active one).
fn dispatch_in_window(cx: &mut App, action: impl gpui::Action) {
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, cx| {
                window.dispatch_action(action.boxed_clone(), cx)
            });
        }
    });
}

/// Activates the app for a window that is about to come forward: the Dock icon
/// and the hidden flag are restored first (a no-op without a tray), so no path
/// returns a window with the Dock icon still gone.
pub fn bring_to_front(cx: &mut App) {
    Tray::set_window_hidden(false, cx);
    cx.activate(true);
}

/// Brings the window forward, creating it if none exists. Through the
/// instance so a headless app leaves headless (and drops its idle quit).
fn show_window(cx: &mut App) {
    // Dock icon first, so the activation below gives the app focus again.
    Tray::set_window_hidden(false, cx);
    use crate::instance::{Instance, ShowFallback};
    if cx.try_global::<Instance>().is_some() || cx.try_global::<ShowFallback>().is_some() {
        Instance::show(cx);
    } else {
        bring_to_front(cx);
    }
}

fn build_view(cx: &App) -> TrayView {
    let l = i18n::gpui::localizer(cx);
    let (rows, paused) = match cx.try_global::<WorkspacesHandle>() {
        Some(h) => {
            let w = h.0.read(cx);
            let paused = w.active_session().is_some_and(|s| {
                let s = s.read(cx);
                s.can_pause() && s.paused(components::now_secs(cx), &TimeZone::UTC).is_some()
            });
            (w.tray_rows(cx), paused)
        }
        None => (Vec::new(), false),
    };
    let badge = rows.iter().map(|r| r.2).sum();
    let ws: Vec<_> = rows
        .iter()
        .map(|(_, name, mentions, expired)| (name.clone(), *mentions, *expired))
        .collect();
    let unread = match cx.try_global::<WorkspacesHandle>() {
        Some(h) => h.0.read(cx).tray_unread(cx),
        None => 0,
    };
    let entries = tray_menu_model(&ws, unread, paused);
    let labels = entries
        .iter()
        .map(|e| match e {
            TrayEntry::Workspace {
                name,
                mentions,
                expired,
                ..
            } => {
                if *expired {
                    t!(l, "tray-workspace-expired", name = name.as_ref()).into_owned()
                } else if *mentions > 0 {
                    t!(
                        l,
                        "tray-workspace-mentions",
                        name = name.as_ref(),
                        count = *mentions
                    )
                    .into_owned()
                } else {
                    name.to_string()
                }
            }
            TrayEntry::Header { unread, mentions } => {
                t!(l, "tray-header", unread = *unread, mentions = *mentions).into_owned()
            }
            TrayEntry::PauseNotifications => t!(l, "tray-pause").into_owned(),
            TrayEntry::Resume => t!(l, "tray-resume").into_owned(),
            TrayEntry::SetStatus => t!(l, "tray-set-status").into_owned(),
            TrayEntry::Settings => t!(l, "tray-settings").into_owned(),
            TrayEntry::Open => t!(l, "tray-open").into_owned(),
            TrayEntry::Quit => t!(l, "tray-quit").into_owned(),
            TrayEntry::Separator => String::new(),
        })
        .collect();
    let pause_items = pause_items(cx, &l);
    TrayView {
        entries,
        labels,
        pause_items,
        keys: rows.into_iter().map(|r| r.0).collect(),
        badge,
        tooltip: if badge > 0 {
            t!(l, "tray-tooltip-mentions", count = badge).into_owned()
        } else {
            t!(l, "tray-tooltip").into_owned()
        },
    }
}

/// The pause submenu: 30 minutes, 1 hour, until tomorrow, custom.
fn pause_items(cx: &App, l: &i18n::Localizer) -> Vec<(String, String)> {
    let (zone, clock) = cx
        .try_global::<WorkspacesHandle>()
        .and_then(|h| h.0.read(cx).active_session())
        .map(|s| {
            let c = s.read(cx).notify_config(cx);
            (c.zone, c.clock)
        })
        .unwrap_or_else(|| {
            let clock = cx
                .try_global::<SettingsStore>()
                .map(|s| s.current().messages.clock)
                .unwrap_or(settings::Clock::H24);
            (TimeZone::system(), clock)
        });
    let now = components::now(cx).to_zoned_datetime(zone);
    components::pause_rows(&now, clock, l)
        .into_iter()
        .filter_map(|row| {
            let id = match row.choice {
                components::PauseChoice::Minutes(m @ (30 | 60)) => format!("pause:{m}"),
                components::PauseChoice::Minutes(_) => return None,
                components::PauseChoice::UntilTomorrow => "pause:tomorrow".to_string(),
                components::PauseChoice::Custom => "pause:custom".to_string(),
            };
            Some((id, row.label))
        })
        .collect()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod system {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use futures::StreamExt as _;
    use gpui::App;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    use super::{Tray, TrayEntry, TrayPort, TrayView, action_for_id, id_of};

    fn decode(png: &[u8]) -> Option<Icon> {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .ok()?;
        let mut buf = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buf).ok()?;
        buf.truncate(info.buffer_size());
        Icon::from_rgba(buf, info.width, info.height).ok()
    }

    /// The real status item. Built on the first model, on the main thread.
    pub struct SystemPort {
        icon: RefCell<Option<TrayIcon>>,
        failed: Cell<bool>,
    }

    impl SystemPort {
        /// Routes menu clicks to the app through one channel; no polling.
        pub fn install(cx: &mut App) -> Rc<Self> {
            let (tx, mut rx) = futures::channel::mpsc::unbounded();
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let _ = tx.unbounded_send(e.id.0);
            }));
            cx.spawn(async move |cx| {
                while let Some(id) = rx.next().await {
                    if let Some(action) = action_for_id(&id) {
                        cx.update(|cx| Tray::dispatch(action, cx));
                    }
                }
            })
            .detach();
            Rc::new(Self {
                icon: RefCell::new(None),
                failed: Cell::new(false),
            })
        }

        fn menu(view: &TrayView) -> Option<Menu> {
            let menu = Menu::new();
            for (entry, label) in view.entries.iter().zip(&view.labels) {
                match (entry, id_of(entry)) {
                    (TrayEntry::PauseNotifications, _) => {
                        let sub = Submenu::new(label, true);
                        for (id, text) in &view.pause_items {
                            sub.append(&MenuItem::with_id(id.as_str(), text, true, None))
                                .ok()?;
                        }
                        menu.append(&sub).ok()?
                    }
                    // The header only reports; it is not clickable.
                    (TrayEntry::Header { .. }, Some(id)) => menu
                        .append(&MenuItem::with_id(id, label, false, None))
                        .ok()?,
                    (_, Some(id)) => menu
                        .append(&MenuItem::with_id(id, label, true, None))
                        .ok()?,
                    (_, None) => menu.append(&PredefinedMenuItem::separator()).ok()?,
                }
            }
            Some(menu)
        }
    }

    impl TrayPort for SystemPort {
        fn show(&self, view: &TrayView, _cx: &mut App) {
            let Some(menu) = Self::menu(view) else { return };
            let Some(glyph) = decode(super::glyph_png(view.badge)) else {
                return;
            };
            let mut slot = self.icon.borrow_mut();
            let Some(tray) = slot.as_ref() else {
                match TrayIconBuilder::new()
                    .with_icon_templated(glyph)
                    .with_menu(Box::new(menu))
                    .with_tooltip(&view.tooltip)
                    .build()
                {
                    Ok(t) => {
                        t.set_title((view.badge > 0).then(|| view.badge.to_string()));
                        *slot = Some(t);
                    }
                    Err(e) => {
                        crate::log::info(&format!("tray icon unavailable: {e}"));
                        self.failed.set(true);
                    }
                }
                return;
            };
            let _ = tray.set_icon_templated(Some(glyph));
            tray.set_menu(Some(Box::new(menu)));
            let _ = tray.set_tooltip(Some(&view.tooltip));
            tray.set_title((view.badge > 0).then(|| view.badge.to_string()));
        }

        fn remove(&self) {
            self.icon.borrow_mut().take();
        }

        fn is_available(&self) -> bool {
            !self.failed.get()
        }
    }
}

/// Installs the real status item where the platform has one. Elsewhere the app
/// has no tray and closing the last window quits, as before.
pub fn install_system(cx: &mut App) {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    Tray::new(system::SystemPort::install(cx)).install(cx);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = cx;
}
