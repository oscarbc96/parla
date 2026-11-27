//! Options of every app window, with the minimum size that keeps its controls
//! reachable (layout tokens in `themes::metrics`).

use gpui::{Bounds, Pixels, WindowBounds, WindowOptions, size};
use themes::metrics as m;

fn base(bounds: Bounds<Pixels>, min: gpui::Size<Pixels>, strip: Pixels) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(min),
        titlebar: crate::titlebar::options(strip),
        ..Default::default()
    }
}

/// The main window.
pub fn main(bounds: Bounds<Pixels>) -> WindowOptions {
    main_on(bounds, None)
}

/// The main window on a given display (`bounds` are relative to it); the
/// primary display when `None`.
pub fn main_on(bounds: Bounds<Pixels>, display: Option<gpui::DisplayId>) -> WindowOptions {
    WindowOptions {
        display_id: display,
        ..base(
            bounds,
            size(m::LAYOUT_WINDOW_MIN_WIDTH, m::LAYOUT_WINDOW_MIN_HEIGHT),
            m::LAYOUT_TITLEBAR,
        )
    }
}

/// The add-workspace sign-in window.
pub fn sign_in(bounds: Bounds<Pixels>) -> WindowOptions {
    base(
        bounds,
        size(m::LAYOUT_SIGN_IN_MIN_WIDTH, m::LAYOUT_SIGN_IN_MIN_HEIGHT),
        crate::titlebar::MAC_TOP_INSET,
    )
}

/// The add-workspace window while it shows the first-run screens: they are laid
/// out for the main window, so it keeps that window's minimum size.
pub fn sign_in_welcome(bounds: Bounds<Pixels>) -> WindowOptions {
    base(
        bounds,
        size(m::LAYOUT_WINDOW_MIN_WIDTH, m::LAYOUT_WINDOW_MIN_HEIGHT),
        crate::titlebar::MAC_TOP_INSET,
    )
}

/// The About window.
pub fn about(bounds: Bounds<Pixels>) -> WindowOptions {
    base(
        bounds,
        size(
            m::LAYOUT_WINDOW_ABOUT_MIN_WIDTH,
            m::LAYOUT_WINDOW_ABOUT_MIN_HEIGHT,
        ),
        crate::titlebar::MAC_TOP_INSET,
    )
}

/// The media viewer; the header strip is the viewer header's height so the
/// traffic lights centre in it.
pub fn media_viewer(bounds: Bounds<Pixels>) -> WindowOptions {
    base(
        bounds,
        size(
            m::LAYOUT_WINDOW_MEDIA_MIN_WIDTH,
            m::LAYOUT_WINDOW_MEDIA_MIN_HEIGHT,
        ),
        m::LAYOUT_MEDIA_VIEWER_HEADER,
    )
}

/// The code viewer; the header strip is the titlebar's height so the traffic
/// lights centre in it.
pub fn code_viewer(bounds: Bounds<Pixels>) -> WindowOptions {
    base(
        bounds,
        size(
            m::LAYOUT_WINDOW_MEDIA_MIN_WIDTH,
            m::LAYOUT_WINDOW_MEDIA_MIN_HEIGHT,
        ),
        m::LAYOUT_TITLEBAR,
    )
}

/// The agent audit log window.
pub fn audit_log(bounds: Bounds<Pixels>) -> WindowOptions {
    base(
        bounds,
        size(
            m::LAYOUT_WINDOW_AUDIT_MIN_WIDTH,
            m::LAYOUT_WINDOW_AUDIT_MIN_HEIGHT,
        ),
        crate::titlebar::MAC_TOP_INSET,
    )
}

/// A pop-out window; `thread` windows have the taller header strip.
pub fn popout(bounds: Bounds<Pixels>, thread: bool) -> WindowOptions {
    base(
        bounds,
        size(
            m::LAYOUT_WINDOW_POPOUT_MIN_WIDTH,
            m::LAYOUT_WINDOW_POPOUT_MIN_HEIGHT,
        ),
        if thread {
            m::LAYOUT_POPOUT_HEADER
        } else {
            crate::titlebar::MAC_TOP_INSET
        },
    )
}

/// The Settings window; the header strip is the Settings header's height so
/// the traffic lights centre in it.
pub fn settings(bounds: Bounds<Pixels>) -> WindowOptions {
    base(
        bounds,
        size(
            m::LAYOUT_WINDOW_SETTINGS_MIN_WIDTH,
            m::LAYOUT_WINDOW_SETTINGS_MIN_HEIGHT,
        ),
        m::LAYOUT_SETTINGS_HEADER,
    )
}
