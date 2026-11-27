//! The main window's saved size and position: clamped to a visible display,
//! written once after a move or resize ends.

use std::time::Duration;

use gpui::{Bounds, TestAppContext, VisualTestContext, point, px, size};
use parla::window_state::{
    DisplayInfo, MainWindow, clamp_to_display, load_main, main_path_in, restore_display, save_main,
    track_main,
};

fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<gpui::Pixels> {
    Bounds {
        origin: point(px(x), px(y)),
        size: size(px(w), px(h)),
    }
}

const MIN: gpui::Size<gpui::Pixels> = gpui::Size {
    width: px(400.0),
    height: px(300.0),
};

fn disp(uuid: &str, w: f32, h: f32) -> DisplayInfo {
    DisplayInfo {
        uuid: Some(uuid.into()),
        visible: b(0.0, 0.0, w, h),
    }
}

#[test]
fn bounds_on_the_display_are_kept() {
    let v = b(0.0, 0.0, 1920.0, 1080.0);
    let saved = b(100.0, 80.0, 1200.0, 800.0);
    assert_eq!(clamp_to_display(saved, v, MIN), Some(saved));
}

#[test]
fn reopens_on_the_saved_display_with_per_display_coordinates() {
    // Every display starts at 0,0: the saved bounds are relative to its own.
    let d = [disp("main", 1920.0, 1080.0), disp("side", 1440.0, 900.0)];
    let (i, picked) = restore_display(Some("side"), &d, 0).unwrap();
    assert_eq!(i, 1);
    let saved = b(200.0, 50.0, 1000.0, 700.0);
    assert_eq!(clamp_to_display(saved, picked.visible, MIN), Some(saved));
    // Clamped against that display only: too big for it, so it shrinks, even
    // though the primary would hold it.
    let big = b(100.0, 40.0, 1800.0, 1000.0);
    assert_eq!(
        clamp_to_display(big, picked.visible, MIN),
        Some(b(0.0, 0.0, 1440.0, 900.0))
    );
}

#[test]
fn a_gone_display_falls_back_to_the_primary() {
    let d = [disp("main", 1920.0, 1080.0), disp("side", 1440.0, 900.0)];
    assert_eq!(restore_display(Some("unplugged"), &d, 0).unwrap().0, 0);
    assert_eq!(restore_display(None, &d, 1).unwrap().0, 1);
    assert!(restore_display(Some("x"), &[], 0).is_none());
}

#[test]
fn partly_off_screen_is_moved_in_and_oversize_shrinks() {
    let v = b(0.0, 0.0, 1440.0, 900.0);
    let saved = b(1000.0, 700.0, 800.0, 600.0);
    assert_eq!(
        clamp_to_display(saved, v, MIN),
        Some(b(640.0, 300.0, 800.0, 600.0))
    );
    let saved = b(-50.0, -20.0, 3000.0, 2000.0);
    assert_eq!(clamp_to_display(saved, v, MIN), Some(v));
    // Only a sliver on the display: not reachable, so not restored.
    assert_eq!(
        clamp_to_display(b(1400.0, 100.0, 800.0, 600.0), v, MIN),
        None
    );
}

#[test]
fn saved_window_round_trips_and_garbage_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = main_path_in(dir.path());
    assert_eq!(load_main(&path), None);
    let saved = MainWindow {
        bounds: b(10.0, 20.0, 1100.0, 750.0),
        display: Some("side".into()),
    };
    save_main(&path, &saved).unwrap();
    assert_eq!(load_main(&path), Some(saved));
    std::fs::write(&path, b"not json").unwrap();
    assert_eq!(load_main(&path), None);
}

#[gpui::test]
async fn bounds_are_written_once_after_the_window_stops_moving(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = main_path_in(dir.path());
    let p = path.clone();
    let w = cx.add_window(move |window, cx| {
        track_main::<Probe>(p, window, cx);
        Probe
    });
    let vcx = VisualTestContext::from_window(w.into(), cx);
    for width in [900.0, 950.0, 1000.0] {
        vcx.simulate_resize(size(px(width), px(700.0)));
        vcx.executor().advance_clock(Duration::from_millis(100));
    }
    assert!(!path.exists(), "nothing is written while the drag goes on");
    vcx.executor().advance_clock(Duration::from_millis(600));
    vcx.run_until_parked();
    let saved = load_main(&path).expect("written after the window rested");
    assert_eq!(saved.bounds.size, size(px(1000.0), px(700.0)));
}

struct Probe;
impl gpui::Render for Probe {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::Empty
    }
}

#[gpui::test]
async fn a_pending_save_is_flushed_when_the_window_is_released(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = main_path_in(dir.path());
    let p = path.clone();
    let w = cx.add_window(move |window, cx| {
        track_main::<Probe>(p, window, cx);
        Probe
    });
    let mut vcx = VisualTestContext::from_window(w.into(), cx);
    vcx.simulate_resize(size(px(900.0), px(650.0)));
    assert!(!path.exists(), "still inside the quiet period");
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();
    let saved = load_main(&path).expect("flushed on close");
    assert_eq!(saved.bounds.size, size(px(900.0), px(650.0)));
}
