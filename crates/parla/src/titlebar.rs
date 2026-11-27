//! Full-bleed window chrome. On macOS the system titlebar is transparent and
//! our content extends to the top edge. The main window draws a 44 px titlebar
//! (back, forward, search pill, pause bell) across the top; every other window
//! reserves a 32 px strip per column that drags the window and zooms it on
//! double-click. [`options`] centres the traffic lights in whichever one the
//! window uses. Elsewhere the native titlebar stays and nothing is reserved.

use std::rc::Rc;

use gpui::{
    App, ClickEvent, Div, ElementId, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Pixels, Point, SharedString, StatefulInteractiveElement as _, Styled as _,
    TitlebarOptions, Window, WindowControlArea, div, point, prelude::FluentBuilder as _, px,
};
use themes::{ActiveTheme as _, parla_tokens};

/// Height of the reserved top strip on macOS (aligned with the traffic lights).
pub const MAC_TOP_INSET: Pixels = themes::metrics::LAYOUT_MAC_TOP_INSET;
/// Left offset of the traffic lights on macOS.
pub const MAC_TRAFFIC_LIGHTS_LEFT: f32 = 16.0;
/// Diameter of a traffic light.
const MAC_LIGHT_SIZE: f32 = 14.0;

/// Top offset that centres the lights in a strip of `strip_height`.
pub fn traffic_lights_top(strip_height: Pixels) -> f32 {
    ((strip_height.as_f32() - MAC_LIGHT_SIZE) / 2.0).round()
}

/// Height of the main window's titlebar (zero off macOS, where the native
/// titlebar stays).
pub fn bar_height() -> Pixels {
    if cfg!(target_os = "macos") {
        themes::metrics::LAYOUT_TITLEBAR
    } else {
        px(0.0)
    }
}

/// Height of the top strip for the current platform (zero off macOS).
pub fn top_inset() -> Pixels {
    px(top_inset_for(cfg!(target_os = "macos")))
}

/// Pure form of [`top_inset`] for tests.
pub fn top_inset_for(macos: bool) -> f32 {
    if macos { MAC_TOP_INSET.as_f32() } else { 0.0 }
}

/// Titlebar options for a window whose top strip is `strip_height` tall: the
/// main window passes [`themes::metrics::LAYOUT_TITLEBAR`], the others
/// [`MAC_TOP_INSET`].
pub fn options(strip_height: Pixels) -> Option<TitlebarOptions> {
    let (x, y) = (MAC_TRAFFIC_LIGHTS_LEFT, traffic_lights_top(strip_height));
    let mac = cfg!(target_os = "macos");
    Some(TitlebarOptions {
        title: Some("Parla".into()),
        appears_transparent: mac,
        traffic_light_position: mac.then(|| -> Point<Pixels> { point(px(x), px(y)) }),
    })
}

/// Makes `d` drag the window and zoom it on double-click. A no-op off macOS.
pub fn draggable(d: Div) -> Div {
    if !cfg!(target_os = "macos") {
        return d;
    }
    d.window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |ev, window, _| {
            if ev.click_count >= 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
}

/// A drag and double-click-to-zoom strip. Empty (zero height) off macOS.
pub fn strip() -> Div {
    draggable(div().w_full().flex_none().h(top_inset()))
}

type Click = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// What the titlebar buttons do.
pub struct BarActions {
    pub back: Click,
    pub forward: Click,
    pub search: Click,
    pub pause: Click,
    pub can_back: bool,
    pub can_forward: bool,
    /// Placeholder of the search pill ("Search <workspace>").
    pub search_label: SharedString,
    pub back_label: SharedString,
    pub forward_label: SharedString,
    pub pause_label: SharedString,
}

/// The main window's titlebar (macOS): drag area with the traffic lights at
/// the left, back and forward, a search pill and the pause-notifications bell.
pub fn bar(actions: BarActions, cx: &App) -> Div {
    let t = parla_tokens(cx.theme());
    let button = |id: &'static str,
                  glyph: Result<components::icon::IconName, &'static str>,
                  label: SharedString,
                  on: Click,
                  enabled: bool| {
        div()
            .id(ElementId::Name(id.into()))
            .debug_selector(move || id.to_string())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(themes::metrics::LAYOUT_TITLEBAR_BUTTON)
            .rounded_md()
            .text_size(themes::metrics::FONT_SIZE_XL)
            .text_color(t.sidebar_muted)
            .when(!enabled, |d| d.opacity(0.45))
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover(|h| h.bg(t.sidebar_hover).text_color(t.sidebar_text_strong))
            })
            .tooltip(components::reactions::tooltip_view(label.to_string()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .when(enabled, |d| d.on_click(move |e, w, cx| on(e, w, cx)))
            .child(match glyph {
                Ok(icon) => components::icon::Icon::new(icon)
                    .size(themes::metrics::ICON_SIZE_LG)
                    .color(t.sidebar_muted)
                    .into_any_element(),
                // The bell is a colour emoji, drawn by the system.
                Err(text) => div().child(text).into_any_element(),
            })
    };
    let search = actions.search.clone();
    div()
        .debug_selector(|| "titlebar".into())
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |ev, window, _| {
            if ev.click_count >= 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
        .flex()
        .flex_none()
        .items_center()
        .gap(themes::metrics::SPACE_3)
        .w_full()
        .h(bar_height())
        .pl(MAC_LIGHTS_CLEARANCE)
        .pr(themes::metrics::SPACE_5)
        .bg(t.sidebar_rail)
        .border_b_1()
        .border_color(t.sidebar_border)
        .text_color(t.sidebar_text)
        .child(button(
            "titlebar-back",
            Ok(components::icon::IconName::ChevLeft),
            actions.back_label,
            actions.back,
            actions.can_back,
        ))
        .child(button(
            "titlebar-forward",
            Ok(components::icon::IconName::ChevRight),
            actions.forward_label,
            actions.forward,
            actions.can_forward,
        ))
        .child(
            div().flex().flex_1().min_w_0().justify_center().child(
                div()
                    .id("titlebar-search")
                    .debug_selector(|| "titlebar-search".into())
                    .flex()
                    .items_center()
                    .w_full()
                    .max_w(themes::metrics::LAYOUT_TITLEBAR_SEARCH)
                    .h(themes::metrics::LAYOUT_TITLEBAR_BUTTON)
                    .px(themes::metrics::SPACE_4)
                    .rounded_md()
                    .border_1()
                    .border_color(t.sidebar_border)
                    .bg(t.sidebar_hover)
                    .text_size(themes::metrics::FONT_SIZE_SM)
                    .text_color(t.sidebar_muted)
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |e, w, cx| search(e, w, cx))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(actions.search_label),
                    ),
            ),
        )
        .child(button(
            "titlebar-pause",
            Err("\u{1F514}"),
            actions.pause_label,
            actions.pause,
            true,
        ))
}

/// Left clearance for the traffic lights (cluster width plus its inset).
const MAC_LIGHTS_CLEARANCE: Pixels = themes::metrics::LAYOUT_TITLEBAR_LIGHTS;

/// A column body without a strip: the titlebar spans the window above it.
pub fn body(content: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(div().flex_1().min_h_0().min_w_0().child(content))
}

/// Wraps a column body under the strip: strip on top, content below.
pub fn column(content: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(strip())
        .child(div().flex_1().min_h_0().min_w_0().child(content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inset_per_platform() {
        assert_eq!(top_inset_for(true), 32.0);
        assert_eq!(top_inset_for(false), 0.0);
    }

    #[test]
    fn traffic_lights_are_centred_in_their_strip() {
        for strip in [themes::metrics::LAYOUT_TITLEBAR, MAC_TOP_INSET] {
            let y = traffic_lights_top(strip);
            let below = strip.as_f32() - (y + MAC_LIGHT_SIZE);
            assert!((y - below).abs() <= 1.0, "strip {strip:?}: {y} vs {below}");
        }
        assert_eq!(traffic_lights_top(themes::metrics::LAYOUT_TITLEBAR), 15.0);
        assert_eq!(traffic_lights_top(MAC_TOP_INSET), 9.0);
    }
}
