//! The menu-bar (tray) icon: the pure menu model, the event-driven model
//! rebuild against `FakeProvider` workspaces, and what closing the window does.
//! The real status item is never created here: a recording port stands in.

mod support;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use domain::ProviderEvent;
use domain::test_support::msg;
use gpui::{BorrowAppContext as _, QuitMode, SharedString, TestAppContext, VisualTestContext};
use parla::tray::{
    Tray, TrayAction, TrayEntry, TrayPort, TrayView, action_for_id, tray_menu_model,
};
use parla::workspaces::WorkspacesHandle;
use settings::DockIcon;
use support::{Env, env, key, settle, workspaces};

fn ws(name: &str, mentions: u32, expired: bool) -> (SharedString, u32, bool) {
    (name.to_string().into(), mentions, expired)
}

#[test]
fn menu_lists_workspaces_with_counts() {
    let m = tray_menu_model(&[ws("Alpha", 3, false), ws("Beta", 0, true)], 5, false);
    assert_eq!(
        m,
        vec![
            TrayEntry::Header {
                unread: 5,
                mentions: 3
            },
            TrayEntry::Separator,
            TrayEntry::Workspace {
                name: "Alpha".into(),
                mentions: 3,
                expired: false,
                index: 0
            },
            TrayEntry::Workspace {
                name: "Beta".into(),
                mentions: 0,
                expired: true,
                index: 1
            },
            TrayEntry::Separator,
            TrayEntry::PauseNotifications,
            TrayEntry::SetStatus,
            TrayEntry::Separator,
            TrayEntry::Settings,
            TrayEntry::Open,
            TrayEntry::Quit,
        ]
    );
}

#[test]
fn paused_shows_resume() {
    let m = tray_menu_model(&[ws("Alpha", 0, false)], 0, true);
    assert!(m.contains(&TrayEntry::Resume));
    assert!(!m.contains(&TrayEntry::PauseNotifications));
    // No workspace: no header, nothing to pause or set a status on, but
    // Settings is still there.
    let none = tray_menu_model(&[], 0, false);
    assert_eq!(
        none,
        vec![TrayEntry::Settings, TrayEntry::Open, TrayEntry::Quit]
    );
}

#[test]
fn menu_ids_map_back_to_actions() {
    assert_eq!(action_for_id("ws:2"), Some(TrayAction::Workspace(2)));
    assert_eq!(
        action_for_id("pause:30"),
        Some(TrayAction::PauseMinutes(30))
    );
    assert_eq!(
        action_for_id("pause:60"),
        Some(TrayAction::PauseMinutes(60))
    );
    assert_eq!(
        action_for_id("pause:tomorrow"),
        Some(TrayAction::PauseTomorrow)
    );
    assert_eq!(action_for_id("pause:custom"), Some(TrayAction::PauseCustom));
    assert_eq!(action_for_id("pause:x"), None);
    assert_eq!(action_for_id("resume"), Some(TrayAction::Resume));
    assert_eq!(action_for_id("status"), Some(TrayAction::SetStatus));
    assert_eq!(action_for_id("settings"), Some(TrayAction::Settings));
    assert_eq!(action_for_id("open"), Some(TrayAction::Open));
    assert_eq!(action_for_id("quit"), Some(TrayAction::Quit));
    assert_eq!(action_for_id("header"), None, "the header is not a command");
    assert_eq!(action_for_id("ws:x"), None);
    assert_eq!(action_for_id("nope"), None);
}

#[gpui::test]
async fn the_view_has_the_header_and_the_four_pause_choices(cx: &mut TestAppContext) {
    let (e, port) = setup(cx, r#"{ "locale": "en" }"#);
    let w = workspaces(cx, &e);
    cx.update(|cx| cx.set_global(WorkspacesHandle(w.clone())));
    cx.update(Tray::request);
    frame(cx);
    let v = port.last().unwrap();
    let header = v
        .entries
        .iter()
        .position(|e| matches!(e, TrayEntry::Header { .. }))
        .expect("a header");
    assert!(v.labels[header].contains("unread"), "{:?}", v.labels);
    let ids: Vec<&str> = v.pause_items.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(
        ids,
        ["pause:30", "pause:60", "pause:tomorrow", "pause:custom"]
    );
    assert!(v.pause_items[0].1.contains("30"), "{:?}", v.pause_items);
}

#[gpui::test]
async fn a_pause_choice_snoozes_the_active_workspace_for_its_minutes(cx: &mut TestAppContext) {
    use domain::test_support::Call as ProviderCallRecord;
    let providers = ["w1", "w2", "w3"]
        .iter()
        .map(|id| {
            std::sync::Arc::new(
                domain::test_support::FakeProvider::new(id, domain::UserId("u1".into()))
                    .with_capabilities(domain::Capabilities {
                        dnd: true,
                        ..Default::default()
                    }),
            )
        })
        .collect();
    let e = support::env_with(cx, r#"{ "locale": "en" }"#, providers);
    cx.update(|cx| {
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
        Tray::new(Rc::new(Recording::default())).install(cx);
    });
    let w = workspaces(cx, &e);
    cx.update(|cx| cx.set_global(WorkspacesHandle(w.clone())));
    let calls =
        || -> Vec<ProviderCallRecord> { e.providers.iter().flat_map(|p| p.calls()).collect() };
    // The provider call is made off the main thread: wait for it, not a
    // fixed time that a busy machine overruns.
    cx.update(|cx| Tray::dispatch(TrayAction::PauseMinutes(30), cx));
    support::settle_until(cx, |_| calls().contains(&ProviderCallRecord::Snooze(30)));
    assert!(
        calls().contains(&ProviderCallRecord::Snooze(30)),
        "{:?}",
        calls()
    );
    cx.update(|cx| Tray::dispatch(TrayAction::PauseTomorrow, cx));
    let tomorrow = || {
        calls().into_iter().find_map(|c| match c {
            ProviderCallRecord::Snooze(m) if m != 30 => Some(m),
            _ => None,
        })
    };
    support::settle_until(cx, |_| tomorrow().is_some());
    let m = tomorrow().expect("a second snooze for the time until tomorrow");
    // Until tomorrow's morning: up to 33 h when clicked just after midnight.
    assert!((1..=33 * 60).contains(&m), "{m}");
}

#[derive(Clone, Default)]
struct Recording(
    Rc<RefCell<Vec<Option<TrayView>>>>,
    Rc<std::cell::Cell<usize>>,
    Rc<RefCell<Vec<bool>>>,
);

impl TrayPort for Recording {
    fn show(&self, view: &TrayView, _cx: &mut gpui::App) {
        self.0.borrow_mut().push(Some(view.clone()));
    }
    fn remove(&self) {
        self.0.borrow_mut().push(None);
    }
    fn hide_app(&self, _cx: &mut gpui::App) {
        self.1.set(self.1.get() + 1);
    }
    fn set_dock(&self, visible: bool) {
        self.2.borrow_mut().push(visible);
    }
}

impl Recording {
    fn last(&self) -> Option<TrayView> {
        self.0.borrow().iter().rev().find_map(|v| v.clone())
    }
}

fn setup(cx: &mut TestAppContext, settings: &str) -> (Env, Recording) {
    let e = env(cx, settings);
    let port = Recording::default();
    let p = port.clone();
    cx.update(|cx| {
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
        Tray::new(Rc::new(p)).install(cx);
    });
    (e, port)
}

fn frame(cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
}

#[gpui::test]
async fn badge_change_updates_model_without_rebuild_storm(cx: &mut TestAppContext) {
    let (e, port) = setup(cx, r#"{ "locale": "en" }"#);
    let w = workspaces(cx, &e);
    cx.update(|cx| cx.set_global(WorkspacesHandle(w.clone())));
    cx.update(Tray::request);
    frame(cx);
    let before = port.last().expect("first model");
    assert_eq!(before.badge, 0);
    assert_eq!(before.keys.len(), 3);
    let rebuilds = cx.update(|cx| cx.global::<Tray>().rebuilds());

    // 100 mentions of workspace w1 in one burst.
    for i in 0..100 {
        e.providers[0].push(ProviderEvent::MessagePosted(msg(
            "D1",
            &format!("{}.000000", 10 + i),
            "ping",
        )));
    }
    settle(cx);
    frame(cx);
    let after = port.last().unwrap();
    assert_eq!(after.badge, 100, "badge is the sum of mentions");
    assert!(
        after.labels.iter().any(|l| l.contains("100")),
        "{:?}",
        after.labels
    );
    let now = cx.update(|cx| cx.global::<Tray>().rebuilds());
    assert!(
        now - rebuilds <= 1,
        "one rebuild for the burst, got {}",
        now - rebuilds
    );
    // Nothing changed: no further rebuild, no timer left behind.
    frame(cx);
    assert_eq!(cx.update(|cx| cx.global::<Tray>().rebuilds()), now);
}

#[gpui::test]
async fn clicking_a_workspace_activates_it(cx: &mut TestAppContext) {
    let (e, port) = setup(cx, r#"{ "locale": "en" }"#);
    let w = workspaces(cx, &e);
    cx.update(|cx| cx.set_global(WorkspacesHandle(w.clone())));
    cx.update(Tray::request);
    frame(cx);
    let view = port.last().unwrap();
    let i = view.keys.iter().position(|k| *k == key("w3")).unwrap();
    cx.update(|cx| Tray::dispatch(TrayAction::Workspace(i), cx));
    cx.run_until_parked();
    assert_eq!(w.read_with(cx, |w, _| w.active().cloned()), Some(key("w3")));
}

#[gpui::test]
async fn setting_tray_off_removes_the_icon(cx: &mut TestAppContext) {
    let (_e, port) = setup(cx, r#"{ "locale": "en", "tray": false }"#);
    cx.update(Tray::request);
    frame(cx);
    assert!(port.last().is_none(), "no icon while the setting is off");
}

#[test]
fn quit_mode_follows_the_setting() {
    assert_eq!(
        parla::tray::quit_mode_for(false),
        QuitMode::LastWindowClosed
    );
    assert_eq!(parla::tray::quit_mode_for(true), QuitMode::Explicit);
}

#[gpui::test]
async fn tray_false_quits_on_last_window(cx: &mut TestAppContext) {
    let (_e, _port) = setup(cx, r#"{ "locale": "en", "tray": false }"#);
    let w = cx.add_window(|_, _| gpui::Empty);
    let mut vcx = VisualTestContext::from_window(w.into(), cx);
    vcx.update(|window, cx| Tray::guard_window(window, cx));
    assert!(vcx.simulate_close(), "the window closes, GPUI then quits");
}

#[gpui::test]
async fn tray_true_keeps_the_window_for_the_icon(cx: &mut TestAppContext) {
    let (_e, port) = setup(cx, r#"{ "locale": "en" }"#);
    let w = cx.add_window(|_, _| gpui::Empty);
    let mut vcx = VisualTestContext::from_window(w.into(), cx);
    vcx.update(|window, cx| Tray::guard_window(window, cx));
    assert!(
        !vcx.simulate_close(),
        "closing hides the app instead, the tray icon brings it back"
    );
    assert_eq!(port.1.get(), 1);
}

#[test]
fn dock_visibility_policy() {
    use parla::platform::dock_visible;
    // Always, or the tray off: the icon never goes away.
    assert!(dock_visible(true, DockIcon::Always, false));
    assert!(dock_visible(false, DockIcon::WithWindow, false));
    // With window: only a hidden window with the tray on hides it.
    assert!(dock_visible(true, DockIcon::WithWindow, true));
    assert!(!dock_visible(true, DockIcon::WithWindow, false));
    // Never: menu bar only, with a window open too.
    assert!(!dock_visible(true, DockIcon::Never, true));
    assert!(!dock_visible(true, DockIcon::Never, false));
    // Without the tray the icon stays, or the app could not be reached.
    assert!(dock_visible(false, DockIcon::Never, true));
    assert!(dock_visible(false, DockIcon::Never, false));
}

#[gpui::test]
async fn closing_hides_the_dock_icon_and_open_restores_it(cx: &mut TestAppContext) {
    let (_e, port) = setup(cx, r#"{ "locale": "en", "dock_icon": "with_window" }"#);
    let w = cx.add_window(|_, _| gpui::Empty);
    let mut vcx = VisualTestContext::from_window(w.into(), cx);
    vcx.update(|window, cx| Tray::guard_window(window, cx));
    assert!(!vcx.simulate_close());
    assert_eq!(*port.2.borrow(), vec![false]);
    vcx.update(|_, cx| Tray::dispatch(TrayAction::Open, cx));
    assert_eq!(*port.2.borrow(), vec![false, true]);
}

#[gpui::test]
async fn dock_icon_always_never_touches_the_dock(cx: &mut TestAppContext) {
    let (_e, port) = setup(cx, r#"{ "locale": "en" }"#);
    let w = cx.add_window(|_, _| gpui::Empty);
    let mut vcx = VisualTestContext::from_window(w.into(), cx);
    vcx.update(|window, cx| Tray::guard_window(window, cx));
    assert!(!vcx.simulate_close());
    vcx.update(|_, cx| Tray::dispatch(TrayAction::Open, cx));
    assert!(port.2.borrow().is_empty());
}

#[gpui::test]
async fn dock_icon_never_applies_live_and_back(cx: &mut TestAppContext) {
    let (e, port) = setup(cx, r#"{ "locale": "en" }"#);
    let w = cx.add_window(|_, _| gpui::Empty);
    let mut vcx = VisualTestContext::from_window(w.into(), cx);
    vcx.update(|_, cx| Tray::set_window_hidden(false, cx));
    assert!(port.2.borrow().is_empty());
    // Menu bar only: gone while a window is open, no restart.
    rewrite_settings(&e, &mut vcx, r#"{ "locale": "en", "dock_icon": "never" }"#);
    assert_eq!(*port.2.borrow(), vec![false]);
    // Opening the window keeps it gone.
    vcx.update(|_, cx| Tray::set_window_hidden(false, cx));
    assert_eq!(*port.2.borrow(), vec![false]);
    // The tray off: the icon comes back so the app stays reachable.
    rewrite_settings(
        &e,
        &mut vcx,
        r#"{ "locale": "en", "dock_icon": "never", "tray": false }"#,
    );
    assert_eq!(*port.2.borrow(), vec![false, true]);
}

/// A port whose status item cannot be created: it says so once it is asked
/// to show it, as the real one does.
#[derive(Clone, Default)]
struct Failing(Rc<std::cell::Cell<bool>>, Rc<RefCell<Vec<bool>>>);

impl TrayPort for Failing {
    fn show(&self, _: &TrayView, _: &mut gpui::App) {
        self.0.set(true);
    }
    fn remove(&self) {}
    fn set_dock(&self, visible: bool) {
        self.1.borrow_mut().push(visible);
    }
    fn is_available(&self) -> bool {
        !self.0.get()
    }
}

#[gpui::test]
async fn a_tray_that_fails_gives_the_dock_icon_and_quit_mode_back(cx: &mut TestAppContext) {
    let e = env(cx, r#"{ "locale": "en", "dock_icon": "never" }"#);
    let port = Failing::default();
    let p = port.clone();
    cx.update(|cx| {
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
        Tray::new(Rc::new(p)).install(cx);
    });
    let _ = &e;
    // Before the icon is shown the tray is assumed fine: menu bar only.
    assert_eq!(*port.1.borrow(), vec![false]);
    cx.update(Tray::request);
    frame(cx);
    // The icon could not be made: the Dock icon is back and closing the last
    // window quits, so the app is never unreachable.
    assert_eq!(*port.1.borrow(), vec![false, true]);
    assert!(!cx.update(|cx| Tray::hides_on_close(cx)));
}

fn rewrite_settings(e: &Env, cx: &mut TestAppContext, json: &str) {
    std::fs::write(e.config.join("settings.json"), json).unwrap();
    cx.update(|cx| {
        cx.update_global::<settings::SettingsStore, _>(|s, _| {
            s.reload();
        });
        Tray::settings_changed(cx);
    });
}

#[gpui::test]
async fn bring_to_front_restores_the_dock_after_close(cx: &mut TestAppContext) {
    let (_e, port) = setup(cx, r#"{ "locale": "en", "dock_icon": "with_window" }"#);
    let w = cx.add_window(|_, _| gpui::Empty);
    let mut vcx = VisualTestContext::from_window(w.into(), cx);
    vcx.update(|window, cx| Tray::guard_window(window, cx));
    assert!(!vcx.simulate_close());
    assert_eq!(*port.2.borrow(), vec![false]);
    // The path every notification click, ring and MCP window takes.
    vcx.update(|_, cx| parla::tray::bring_to_front(cx));
    assert_eq!(*port.2.borrow(), vec![false, true]);
}

#[gpui::test]
async fn settings_change_while_hidden_resyncs_the_dock(cx: &mut TestAppContext) {
    let (e, port) = setup(cx, r#"{ "locale": "en", "dock_icon": "with_window" }"#);
    let w = cx.add_window(|_, _| gpui::Empty);
    let mut vcx = VisualTestContext::from_window(w.into(), cx);
    vcx.update(|window, cx| Tray::guard_window(window, cx));
    assert!(!vcx.simulate_close());
    assert_eq!(*port.2.borrow(), vec![false]);
    rewrite_settings(&e, cx, r#"{ "locale": "en", "dock_icon": "always" }"#);
    assert_eq!(*port.2.borrow(), vec![false, true]);
    rewrite_settings(&e, cx, r#"{ "locale": "en", "dock_icon": "with_window" }"#);
    assert_eq!(*port.2.borrow(), vec![false, true, false]);
}

#[gpui::test]
async fn tray_off_with_window_dock_never_hides(cx: &mut TestAppContext) {
    let (_e, port) = setup(
        cx,
        r#"{ "locale": "en", "tray": false, "dock_icon": "with_window" }"#,
    );
    let w = cx.add_window(|_, _| gpui::Empty);
    let mut vcx = VisualTestContext::from_window(w.into(), cx);
    vcx.update(|window, cx| Tray::guard_window(window, cx));
    // The window really closes (no tray to bring it back) and the Dock stays.
    assert!(vcx.simulate_close());
    vcx.update(|_, cx| parla::tray::bring_to_front(cx));
    assert!(port.2.borrow().is_empty());
}

/// Decodes a glyph PNG: (width, height, alpha plane, whether any lit pixel is not black).
fn decode_glyph(png_bytes: &[u8]) -> (u32, u32, Vec<u8>, bool) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes))
        .read_info()
        .unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let alpha: Vec<u8> = buf.chunks(4).map(|p| p[3]).collect();
    let coloured = buf.chunks(4).any(|p| p[3] > 0 && (p[0] | p[1] | p[2]) != 0);
    (info.width, info.height, alpha, coloured)
}

/// The status item draws a custom icon at its pixel size in points and scales
/// anything taller than 22 pt down to 22 pt. So the canvas is 44 px (22 pt at
/// 2x) and the bubble sits inside padding at 14 to 18 pt, like other items.
#[test]
fn menu_bar_glyphs_are_template_images_sized_like_status_items() {
    for badge in [0, 3] {
        let (w, h, alpha, coloured) = decode_glyph(parla::tray::glyph_png(badge));
        assert_eq!((w, h), (44, 44), "2x canvas of the 22 pt status item");
        assert!(!coloured, "a template image is black; only alpha matters");
        let lit = |x: u32, y: u32| alpha[(y * w + x) as usize] > 0;
        let xs: Vec<u32> = (0..w).filter(|&x| (0..h).any(|y| lit(x, y))).collect();
        let ys: Vec<u32> = (0..h).filter(|&y| (0..w).any(|x| lit(x, y))).collect();
        let (gw, gh) = (
            xs.last().unwrap() - xs[0] + 1,
            ys.last().unwrap() - ys[0] + 1,
        );
        // 28 to 36 px is 14 to 18 pt.
        assert!((28..=36).contains(&gh), "glyph {gh}px high");
        assert!((28..=36).contains(&gw), "glyph {gw}px wide");
        assert!(xs[0] >= 4 && ys[0] >= 4, "padding left and top");
        assert!(
            xs.last().unwrap() + 4 < w && ys.last().unwrap() + 4 < h,
            "padding right and bottom"
        );
    }
    assert_ne!(parla::tray::glyph_png(0), parla::tray::glyph_png(1));
}
