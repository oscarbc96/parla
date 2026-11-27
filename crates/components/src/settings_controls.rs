//! Controls of the Settings window (the SettingsGeneral, SettingsAppearance
//! and SettingsNotifications boards): switch, segmented control, choice card,
//! radio card, theme and density cards, chip, keyword field, time field,
//! select, notice, and the section and row layout. Presentation only: the page wires activation with
//! [`crate::controls::on_activate_app`]. Every control is a keyboard tab stop
//! with the focus ring and carries a `debug_selector` equal to its id.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, Div, Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Pixels, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::*, relative,
};
use themes::{ThemeColors, metrics};

use crate::controls::focus_style;
use crate::text_input::TextInput;

/// A control frame that takes the keyboard focus and shows the ring.
fn focusable(id: &SharedString, colors: &ThemeColors) -> Stateful<Div> {
    let selector = id.to_string();
    div()
        .id(id.clone())
        .debug_selector(move || selector.clone())
        .cursor_pointer()
        .tab_stop(true)
        .focusable()
        .focus_visible(focus_style(colors))
}

/// An on/off switch: a 34 by 20 track, accent when on, with a thumb.
pub fn switch(id: impl Into<SharedString>, on: bool, colors: &ThemeColors) -> Stateful<Div> {
    let id = id.into();
    let thumb = div()
        .size(metrics::SPACE_6)
        .rounded(metrics::RADIUS_FULL)
        .bg(if on {
            colors.text_on_accent
        } else {
            colors.surface_background
        });
    focusable(&id, colors)
        .flex()
        .flex_none()
        .items_center()
        .px(metrics::SPACE_1)
        .w(metrics::LAYOUT_SWITCH_WIDTH)
        .h(metrics::SPACE_7)
        .rounded(metrics::RADIUS_FULL)
        .bg(if on {
            colors.element_background
        } else {
            colors.border_strong
        })
        .when(on, |d| d.justify_end())
        .child(thumb)
}

/// The sunken frame around the segments of a segmented control.
pub fn segmented(segments: impl IntoIterator<Item = Stateful<Div>>, colors: &ThemeColors) -> Div {
    div()
        .flex()
        .flex_none()
        .gap(metrics::SPACE_1)
        .p(metrics::SPACE_1)
        .rounded(metrics::RADIUS_MD)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_background)
        .children(segments)
}

/// [`segmented`] as a labelled radio group, for screen readers: `label`
/// names the choice (the row's label).
pub fn segmented_group(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    segments: impl IntoIterator<Item = Stateful<Div>>,
    colors: &ThemeColors,
) -> Stateful<Div> {
    segmented(segments, colors)
        .id(id.into())
        .role(gpui::Role::RadioGroup)
        .aria_label(label)
}

/// One segment; the selected one is raised on the surface colour.
pub fn segment(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    segment_frame(id, selected, colors).child(label.into())
}

/// A [`segment`] whose label holds key symbols (drawn as icons, spoken by name).
pub fn segment_keys(
    id: impl Into<SharedString>,
    label: &crate::keycap::Glyphs,
    selected: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let color = if selected {
        colors.text
    } else {
        colors.text_muted
    };
    segment_frame(id, selected, colors).child(label.row(metrics::ICON_SIZE_SM, color))
}

fn segment_frame(
    id: impl Into<SharedString>,
    selected: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let id = id.into();
    focusable(&id, colors)
        .flex()
        .items_center()
        .h(metrics::LAYOUT_ROW_COMPACT)
        .px(metrics::SPACE_5)
        .rounded(metrics::RADIUS_SM)
        .text_size(metrics::FONT_SIZE_XS)
        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
        .text_color(if selected {
            colors.text
        } else {
            colors.text_muted
        })
        .when(selected, |d| d.bg(colors.surface_background))
        .when(!selected, |d| d.hover(|h| h.bg(colors.element_hover)))
        .role(gpui::Role::RadioButton)
        .aria_selected(selected)
}

/// A pill that picks one value (the language shortcuts).
pub fn chip(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    on: bool,
    colors: &ThemeColors,
    accent_text: gpui::Hsla,
) -> Stateful<Div> {
    let id = id.into();
    focusable(&id, colors)
        .flex()
        .items_center()
        .h(metrics::LAYOUT_ROW_COMPACT)
        .px(metrics::SPACE_5)
        .rounded(metrics::RADIUS_FULL)
        .border_1()
        .text_size(metrics::FONT_SIZE_XS)
        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
        .when(on, |d| {
            d.bg(colors.accent_soft)
                .text_color(accent_text)
                .border_color(gpui::transparent_black())
        })
        .when(!on, |d| {
            d.bg(colors.panel_background)
                .text_color(colors.text_muted)
                .border_color(colors.border)
        })
        .child(label.into())
}

/// A square weekday tile with a one-letter label. `name` is the full day
/// name for assistive tech, so the letters (T, S) are not ambiguous; `on`
/// is exposed as the toggled state.
pub fn day_tile(
    id: impl Into<SharedString>,
    letter: impl Into<SharedString>,
    name: impl Into<SharedString>,
    on: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let id = id.into();
    let name: SharedString = name.into();
    focusable(&id, colors)
        .role(gpui::Role::Button)
        .aria_label(name.clone())
        .aria_toggled(if on {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .tooltip(crate::reactions::tooltip_shared(name))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(metrics::LAYOUT_DAY_TILE)
        .rounded(metrics::RADIUS_MD)
        .border_1()
        .text_size(metrics::FONT_SIZE_XS)
        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
        .when(on, |d| {
            d.bg(colors.element_background)
                .text_color(colors.text_on_accent)
                .border_color(gpui::transparent_black())
        })
        .when(!on, |d| {
            d.bg(colors.surface_background)
                .text_color(colors.text_muted)
                .border_color(colors.border)
        })
        .child(letter.into())
}

/// A selectable card with a title and a one-line description; the selected
/// one has the accent ring.
pub fn choice_card(
    id: impl Into<SharedString>,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    selected: bool,
    colors: &ThemeColors,
    muted: gpui::Hsla,
) -> Stateful<Div> {
    let id = id.into();
    focusable(&id, colors)
        .flex()
        .flex_col()
        .flex_1()
        .gap(metrics::SPACE_4)
        .p(metrics::SPACE_5)
        .rounded(metrics::RADIUS_LG)
        .border_1()
        .border_color(if selected {
            colors.element_background
        } else {
            colors.border
        })
        .bg(colors.surface_background)
        .when(selected, |d| {
            d.shadow(vec![gpui::BoxShadow {
                color: colors.element_background,
                offset: gpui::point(gpui::px(0.0), gpui::px(0.0)),
                blur_radius: gpui::px(0.0),
                spread_radius: metrics::SPACE_1,
                inset: false,
            }])
        })
        .text_size(metrics::FONT_SIZE_SM)
        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
        .text_color(colors.text)
        .child(title.into())
        .child(
            div()
                .text_size(metrics::FONT_SIZE_XS)
                .font_weight(metrics::FONT_WEIGHT_REGULAR)
                .text_color(muted)
                .child(description.into()),
        )
}

/// The closed select: the current value and a chevron. `open` shows the ring
/// of the border while its list is shown.
pub fn select_trigger(
    id: impl Into<SharedString>,
    value: impl Into<SharedString>,
    open: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    select_trigger_with(id, None, value, open, colors)
}

/// [`select_trigger`] with an icon before the value (the onboarding language).
pub fn select_trigger_with(
    id: impl Into<SharedString>,
    icon: Option<crate::icon::IconName>,
    value: impl Into<SharedString>,
    open: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let id = id.into();
    let value: SharedString = value.into();
    let label = div()
        .flex()
        .items_center()
        .gap(metrics::SPACE_3)
        .children(icon.map(|i| {
            crate::icon::Icon::new(i)
                .size(metrics::ICON_SIZE_SM)
                .color(colors.text_muted)
        }))
        .child(value);
    focusable(&id, colors)
        .flex()
        .items_center()
        .justify_between()
        .gap(metrics::SPACE_4)
        .h(metrics::LAYOUT_ROW_COZY)
        .min_w(metrics::LAYOUT_FIELD_MD)
        .px(metrics::SPACE_5)
        .rounded(metrics::RADIUS_MD)
        .border_1()
        .border_color(if open {
            colors.border_focused
        } else {
            colors.border_strong
        })
        .bg(colors.surface_background)
        .text_size(metrics::FONT_SIZE_SM)
        .text_color(colors.text)
        .child(label)
        .child(
            crate::icon::Icon::new(crate::icon::IconName::ChevDown)
                .size(metrics::ICON_SIZE_SM)
                .color(colors.text_muted),
        )
}

/// One row of an open select list; the current one carries a check mark.
pub fn select_option(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let id = id.into();
    focusable(&id, colors)
        .flex()
        .items_center()
        .justify_between()
        .h(metrics::LAYOUT_MENU_ROW)
        .px(metrics::SPACE_5)
        .rounded(metrics::RADIUS_SM)
        .text_size(metrics::FONT_SIZE_SM)
        .text_color(colors.text)
        .hover(|h| h.bg(colors.element_hover))
        .when(selected, |d| d.bg(colors.element_selected))
        .child(label.into())
        .children(selected.then(|| {
            crate::icon::Icon::new(crate::icon::IconName::Check)
                .size(metrics::ICON_SIZE_SM)
                .color(colors.text)
        }))
}

/// The list of an open select: a raised popover with the theme's border and
/// shadow.
pub fn select_list(options: impl IntoIterator<Item = Stateful<Div>>, colors: &ThemeColors) -> Div {
    div()
        .flex()
        .flex_col()
        .p(metrics::SPACE_2)
        .min_w(metrics::LAYOUT_FIELD_MD)
        .rounded(metrics::RADIUS_MD)
        .border_1()
        .border_color(colors.border)
        .bg(colors.elevated_surface_background)
        .shadow_md()
        // Presses on the list stop here, not on the rows it covers.
        .occlude()
        .children(options)
}

/// What a select keeps between frames: focus handles for its trigger and
/// options, and where the trigger was last drawn.
struct SelectState {
    trigger: FocusHandle,
    options: Vec<FocusHandle>,
    trigger_bounds: Rc<Cell<Bounds<Pixels>>>,
}

/// A select: `trigger` (from [`select_trigger`]) and, while `options` is
/// given, its list floating over the page. The list hangs under the
/// trigger, right-aligned with it, or sits above it when the window has no
/// room below; it never moves the rows around it. A press outside the list
/// and the trigger, or Escape, calls `on_dismiss`; the caller closes it after
/// a pick. Down and Tab walk into and through the options, Up and Shift-Tab
/// back to the trigger, and the focus returns to the trigger on close.
/// The list carries the debug selector `<id>-list`.
pub fn select(
    id: impl Into<SharedString>,
    trigger: Stateful<Div>,
    options: Option<Vec<Stateful<Div>>>,
    colors: &ThemeColors,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let id = id.into();
    let state = window.use_keyed_state(SharedString::from(format!("{id}-select")), cx, |_, cx| {
        SelectState {
            trigger: cx.focus_handle().tab_stop(true),
            options: Vec::new(),
            trigger_bounds: Rc::default(),
        }
    });
    let n = options.as_ref().map_or(0, Vec::len);
    state.update(cx, |s, cx| {
        while s.options.len() < n {
            s.options.push(cx.focus_handle());
        }
    });
    let (trigger_focus, focus, bounds) = {
        let s = state.read(cx);
        (
            s.trigger.clone(),
            s.options[..n].to_vec(),
            s.trigger_bounds.clone(),
        )
    };
    let record = bounds.clone();
    let root = div()
        .relative()
        .flex_none()
        .child(trigger.track_focus(&trigger_focus))
        // Where the trigger is, so a press on it toggles the list instead of
        // closing and reopening it.
        .child(
            gpui::canvas(move |b, _, _| record.set(b), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        );
    let Some(options) = options else {
        return root;
    };
    let on_dismiss = Rc::new(on_dismiss);
    let keys = {
        let (trigger_focus, focus, on_dismiss) =
            (trigger_focus.clone(), focus.clone(), on_dismiss.clone());
        move |ev: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            let k = &ev.keystroke;
            let at = if trigger_focus.is_focused(window) {
                None
            } else if let Some(i) = focus.iter().position(|f| f.is_focused(window)) {
                Some(i)
            } else {
                return;
            };
            let tab = k.key == "tab";
            let forward = k.key == "down" || (tab && !k.modifiers.shift);
            let back = k.key == "up" || (tab && k.modifiers.shift);
            if k.key == "escape" {
                on_dismiss(window, cx);
                window.focus(&trigger_focus, cx);
            } else if forward {
                let next = at.map_or(0, |i| i + 1);
                match focus.get(next) {
                    Some(f) => window.focus(f, cx),
                    None if tab => {
                        // Past the last option: close and let Tab move on.
                        on_dismiss(window, cx);
                        window.focus(&trigger_focus, cx);
                        return;
                    }
                    None => {}
                }
            } else if back {
                match at {
                    Some(0) => window.focus(&trigger_focus, cx),
                    Some(i) => window.focus(&focus[i - 1], cx),
                    None if tab => {
                        on_dismiss(window, cx);
                        return;
                    }
                    None => {}
                }
            } else {
                return;
            }
            cx.stop_propagation();
        }
    };
    let options = options.into_iter().zip(&focus).map(|(el, f)| {
        let to_trigger = trigger_focus.clone();
        el.track_focus(f)
            .on_click(move |_, window, cx| window.focus(&to_trigger, cx))
    });
    let back_on_pick = trigger_focus.clone();
    let list = select_list(options, colors)
        .id(SharedString::from(format!("{id}-list")))
        .debug_selector({
            let selector = format!("{id}-list");
            move || selector.clone()
        })
        .on_mouse_down_out(move |_, window, cx| {
            if !bounds.get().contains(&window.mouse_position()) {
                on_dismiss(window, cx);
            }
        });
    root.on_key_down(keys)
        .capture_key_down(move |ev: &KeyDownEvent, window, cx| {
            // Enter or Space picks the focused option and closes the list;
            // the focus goes back to the trigger once the pick has run.
            let k = ev.keystroke.key.as_str();
            if matches!(k, "enter" | "space") && !back_on_pick.is_focused(window) {
                let f = back_on_pick.clone();
                window.defer(cx, move |window, cx| window.focus(&f, cx));
            }
        })
        .child(
            // A zero-size box at the trigger's top right; the list hangs one
            // row below it, or flips to end at it when there is no room.
            div().absolute().top_0().right_0().child(
                gpui::deferred(
                    gpui::anchored()
                        .anchor(gpui::Anchor::TopRight)
                        .offset(gpui::point(gpui::px(0.), metrics::LAYOUT_ROW_COZY))
                        .child(div().py(metrics::SPACE_1).occlude().child(list)),
                )
                .priority(1),
            ),
        )
}

/// A label with an optional line of help under it, and its control at the
/// right.
pub fn settings_row(
    label: impl Into<SharedString>,
    help: Option<SharedString>,
    control: impl IntoElement,
    colors: &ThemeColors,
    subtle: gpui::Hsla,
) -> Div {
    let help = help.map(|h| {
        div()
            .text_size(metrics::FONT_SIZE_XS)
            .text_color(subtle)
            .child(h)
            .into_any_element()
    });
    settings_row_with(label, help, control, colors)
}

/// A [`settings_row`] whose help holds key symbols (drawn as icons, spoken by name).
pub fn settings_row_keys(
    label: impl Into<SharedString>,
    help: &crate::keycap::Glyphs,
    control: impl IntoElement,
    colors: &ThemeColors,
    subtle: gpui::Hsla,
) -> Div {
    let help = help
        .row(metrics::ICON_SIZE_SM, subtle)
        .flex_wrap()
        .text_size(metrics::FONT_SIZE_XS)
        .text_color(subtle)
        .into_any_element();
    settings_row_with(label, Some(help), control, colors)
}

fn settings_row_with(
    label: impl Into<SharedString>,
    help: Option<gpui::AnyElement>,
    control: impl IntoElement,
    colors: &ThemeColors,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(metrics::SPACE_5)
        .min_h(metrics::LAYOUT_ROW_COZY)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .text_size(metrics::FONT_SIZE_SM)
                .text_color(colors.text)
                .child(label.into())
                .children(help),
        )
        .child(control)
}

/// A page section: title and description at the left, the controls at the
/// right, a hairline above every section but the first.
pub fn settings_section(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    body: impl IntoElement,
    first: bool,
    colors: &ThemeColors,
    subtle: gpui::Hsla,
    hairline: gpui::Hsla,
) -> Div {
    div()
        .flex()
        .gap(metrics::SPACE_8)
        .py(metrics::SPACE_7)
        .when(!first, |d| d.border_t_1().border_color(hairline))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .w(metrics::LAYOUT_SETTINGS_LABEL)
                .gap(metrics::SPACE_2)
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_SM)
                        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                        .text_color(colors.text)
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_XS)
                        .text_color(subtle)
                        .child(description.into()),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .max_w(metrics::LAYOUT_SETTINGS_CONTENT_MAX)
                .gap(metrics::SPACE_4)
                .child(body),
        )
}

/// One entry of the Settings navigation; the selected page is washed with the
/// selection colour and set in semibold.
pub fn nav_item(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let id = id.into();
    focusable(&id, colors)
        .flex()
        .items_center()
        .h(metrics::SPACE_9)
        .px(metrics::SPACE_5)
        .rounded(metrics::RADIUS_SM)
        .text_size(metrics::FONT_SIZE_SM)
        .when(selected, |d| {
            d.bg(colors.element_selected)
                .text_color(colors.text)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
        })
        .when(!selected, |d| {
            d.text_color(colors.text_muted)
                .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                .hover(|h| h.bg(colors.element_hover))
        })
        .child(label.into())
}

/// Colours of the miniature window on a theme card, read from the theme the
/// card stands for (not from the active one).
#[derive(Clone, Copy, Debug)]
pub struct ThemePreview {
    pub background: gpui::Hsla,
    pub sidebar: gpui::Hsla,
    pub sidebar_text: gpui::Hsla,
    pub text: gpui::Hsla,
    pub accent: gpui::Hsla,
}

/// A theme card: a miniature window in the theme's own colours, its name and
/// whether it is light or dark. The selected one has the accent ring.
pub fn theme_card(
    id: impl Into<SharedString>,
    name: impl Into<SharedString>,
    kind: impl Into<SharedString>,
    selected: bool,
    preview: ThemePreview,
    colors: &ThemeColors,
    subtle: gpui::Hsla,
) -> Stateful<Div> {
    let id = id.into();
    let bar = |color: gpui::Hsla, width: f32, alpha: f32| {
        div()
            .h(metrics::SPACE_2)
            .w(relative(width))
            .rounded(metrics::RADIUS_XS)
            .bg(color.opacity(alpha))
    };
    let sidebar = div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(metrics::SPACE_2)
        .w(metrics::LAYOUT_THEME_PREVIEW_SIDEBAR)
        .p(metrics::SPACE_3)
        .bg(preview.sidebar)
        .child(bar(preview.sidebar_text, 1.0, 0.55))
        .child(bar(preview.sidebar_text, 1.0, 0.55))
        .child(bar(preview.accent, 1.0, 1.0))
        .child(bar(preview.sidebar_text, 1.0, 0.55));
    let main = div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(metrics::SPACE_3)
        .p(metrics::SPACE_4)
        .child(bar(preview.text, 0.6, 1.0))
        .child(bar(preview.text, 0.85, 0.4))
        .child(bar(preview.text, 0.7, 0.4))
        .child(div().flex_1())
        .child(
            div()
                .self_end()
                .w(metrics::SPACE_8)
                .h(metrics::SPACE_4)
                .rounded(metrics::RADIUS_XS)
                .bg(preview.accent),
        );
    focusable(&id, colors)
        .flex()
        .flex_col()
        .flex_none()
        .gap(metrics::SPACE_3)
        .w(metrics::LAYOUT_THEME_CARD_WIDTH)
        .p(metrics::SPACE_3)
        .rounded(metrics::RADIUS_LG)
        .border_1()
        .border_color(if selected {
            colors.element_background
        } else {
            colors.border
        })
        .bg(colors.surface_background)
        .when(selected, |d| d.shadow(accent_ring(colors)))
        .child(
            div()
                .flex()
                .h(metrics::LAYOUT_THEME_PREVIEW_HEIGHT)
                .rounded(metrics::RADIUS_SM)
                .overflow_hidden()
                .border_1()
                .border_color(colors.border)
                .bg(preview.background)
                .child(sidebar)
                .child(main),
        )
        .child(
            div()
                .flex()
                // The kind label sits on the first line of a wrapped name.
                .items_start()
                .gap(metrics::SPACE_3)
                .px(metrics::SPACE_1)
                .pb(metrics::SPACE_1)
                .text_size(metrics::FONT_SIZE_XS)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(colors.text)
                // A long name (Parla High Contrast Light, a user's own) wraps
                // instead of running into the kind label.
                .child(div().flex_1().min_w_0().child(name.into()))
                .child(
                    div()
                        .flex_none()
                        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                        .text_color(subtle)
                        .child(kind.into()),
                ),
        )
}

/// How much an unavailable control's colours are dimmed.
const DISABLED: f32 = 0.4;

/// An accent preset swatch: a round chip in `fill` with the preset name under
/// it. The selected one has a ring in the text colour and a check mark in
/// `text_on_accent` (white on the light fills, near black on the dark ones).
/// A disabled one dims its fill, ring, check and label with
/// [`themes::color::faded`]: element opacity would let the ring behind the
/// fill and the check over it show through.
pub fn accent_swatch(
    id: impl Into<SharedString>,
    name: impl Into<SharedString>,
    fill: gpui::Hsla,
    selected: bool,
    enabled: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let id = id.into();
    let fade = |colour: gpui::Hsla| {
        if enabled {
            colour
        } else {
            themes::color::faded(colour, colors.background, DISABLED)
        }
    };
    let chip = div()
        .flex()
        .items_center()
        .justify_center()
        .size(metrics::LAYOUT_ACCENT_SWATCH)
        .rounded(metrics::RADIUS_FULL)
        .bg(fade(fill))
        .text_color(colors.text_on_accent)
        .when(selected, |d| {
            d.child(
                crate::icon::Icon::new(crate::icon::IconName::Check)
                    .size(metrics::ICON_SIZE_MD)
                    .color(fade(colors.text_on_accent)),
            )
            .shadow(vec![
                gpui::BoxShadow {
                    color: fade(colors.background),
                    offset: gpui::point(gpui::px(0.0), gpui::px(0.0)),
                    blur_radius: gpui::px(0.0),
                    spread_radius: metrics::SPACE_1,
                    inset: false,
                },
                gpui::BoxShadow {
                    color: fade(colors.text),
                    offset: gpui::point(gpui::px(0.0), gpui::px(0.0)),
                    blur_radius: gpui::px(0.0),
                    spread_radius: metrics::SPACE_2,
                    inset: false,
                },
            ])
        });
    focusable(&id, colors)
        .flex()
        .flex_col()
        .items_center()
        .gap(metrics::SPACE_3)
        .p(metrics::SPACE_2)
        .rounded(metrics::RADIUS_SM)
        .child(chip)
        .child(
            div()
                .text_size(metrics::FONT_SIZE_XS)
                .font_weight(if selected {
                    metrics::FONT_WEIGHT_SEMIBOLD
                } else {
                    metrics::FONT_WEIGHT_MEDIUM
                })
                .text_color(fade(colors.text_muted))
                .child(name.into()),
        )
}

/// `el` shown as unavailable: out of the tab order, no pointer, and
/// described as unavailable for assistive tech. Nothing is wired to it. It
/// does not fade the element: its caller dims the element's own colours,
/// because GPUI Fast multiplies element opacity into every paint and
/// overlapping children (a check over its fill, a ring behind it) would show
/// through each other.
pub fn disabled(el: Stateful<Div>, l: &i18n::Localizer) -> Stateful<Div> {
    el.tab_stop(false)
        .cursor_default()
        .aria_description(crate::a11y::unavailable(l))
}

/// The 2 px accent ring of a selected card, outside its border.
fn accent_ring(colors: &ThemeColors) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: colors.element_background,
        offset: gpui::point(gpui::px(0.0), gpui::px(0.0)),
        blur_radius: gpui::px(0.0),
        spread_radius: metrics::SPACE_1,
        inset: false,
    }]
}

/// A density card: its title over `rows` sample list rows, tall for cozy and
/// short for compact.
pub fn density_card(
    id: impl Into<SharedString>,
    title: impl Into<SharedString>,
    rows: usize,
    cozy: bool,
    selected: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let id = id.into();
    let widths = [0.55_f32, 0.4, 0.65];
    let mut list = div().flex().flex_col().gap(metrics::SPACE_4);
    for w in widths.into_iter().cycle().take(rows) {
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(metrics::SPACE_3)
                .when(cozy, |d| d.py(metrics::SPACE_1))
                .child(
                    div()
                        .flex_none()
                        .size(if cozy {
                            metrics::SPACE_7
                        } else {
                            metrics::SPACE_5
                        })
                        .rounded(metrics::RADIUS_XS)
                        .bg(colors.border_strong),
                )
                .child(
                    div()
                        .h(metrics::SPACE_3)
                        .w(relative(w))
                        .rounded(metrics::RADIUS_XS)
                        .bg(colors.border_strong),
                ),
        );
    }
    focusable(&id, colors)
        .flex()
        .flex_col()
        .flex_none()
        .gap(metrics::SPACE_4)
        .w(metrics::LAYOUT_DENSITY_CARD_WIDTH)
        .p(metrics::SPACE_5)
        .rounded(metrics::RADIUS_LG)
        .border_1()
        .border_color(if selected {
            colors.element_background
        } else {
            colors.border
        })
        .bg(colors.surface_background)
        .when(selected, |d| d.shadow(accent_ring(colors)))
        .text_size(metrics::FONT_SIZE_SM)
        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
        .text_color(colors.text)
        .child(title.into())
        .child(list)
}

/// A radio option: a radio mark beside a title and one line of description;
/// the selected one has the accent ring.
pub fn radio_card(
    id: impl Into<SharedString>,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    selected: bool,
    colors: &ThemeColors,
    muted: gpui::Hsla,
) -> Stateful<Div> {
    let id = id.into();
    let radio = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .mt(metrics::SPACE_1)
        .size(metrics::SPACE_6)
        .rounded(metrics::RADIUS_FULL)
        .border_1()
        .border_color(if selected {
            colors.element_background
        } else {
            colors.border_strong
        })
        .when(selected, |d| {
            d.child(
                div()
                    .size(metrics::SPACE_4)
                    .rounded(metrics::RADIUS_FULL)
                    .bg(colors.element_background),
            )
        });
    focusable(&id, colors)
        .flex()
        .flex_1()
        .gap(metrics::SPACE_4)
        .p(metrics::SPACE_5)
        .rounded(metrics::RADIUS_LG)
        .border_1()
        .border_color(if selected {
            colors.element_background
        } else {
            colors.border
        })
        .bg(colors.surface_background)
        .when(selected, |d| d.shadow(accent_ring(colors)))
        .child(radio)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .text_size(metrics::FONT_SIZE_SM)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(colors.text)
                .child(title.into())
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_XS)
                        .font_weight(metrics::FONT_WEIGHT_REGULAR)
                        .text_color(muted)
                        .child(description.into()),
                ),
        )
}

/// A keyword chip: the word and a cross; activating it removes the word.
pub fn keyword_chip(
    id: impl Into<SharedString>,
    word: &str,
    colors: &ThemeColors,
    accent_text: gpui::Hsla,
) -> Stateful<Div> {
    chip(id, format!("{word} \u{d7}"), true, colors, accent_text)
}

/// The keyword box: the chips, then the field that adds the next one.
pub fn keyword_field(
    chips: impl IntoIterator<Item = Stateful<Div>>,
    input: Entity<TextInput>,
    focused: bool,
    colors: &ThemeColors,
    border: gpui::Hsla,
) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(metrics::SPACE_3)
        .p(metrics::SPACE_3)
        .rounded(metrics::RADIUS_MD)
        .border_1()
        .border_color(border)
        .bg(colors.surface_background)
        .text_size(metrics::FONT_SIZE_SM)
        .when(focused, |d| {
            d.border_color(gpui::transparent_black())
                .shadow(crate::controls::ring(colors))
        })
        .children(chips)
        .child(
            div()
                .debug_selector(|| "settings-keyword-input".to_string())
                .flex_1()
                .min_w(metrics::LAYOUT_FIELD_SM)
                .px(metrics::SPACE_3)
                .child(input),
        )
}

/// An HH:MM field around a text input; `border` is `border_control`, or the
/// error colour while the text is not a time.
pub fn time_field(
    id: impl Into<SharedString>,
    input: Entity<TextInput>,
    focused: bool,
    colors: &ThemeColors,
    border: gpui::Hsla,
) -> Div {
    let selector = id.into().to_string();
    crate::controls::field_frame(focused, colors, border)
        .debug_selector(move || selector.clone())
        .flex_none()
        .w(metrics::LAYOUT_TIME_FIELD_WIDTH)
        .text_size(metrics::FONT_SIZE_SM)
        .child(input)
}

/// `text` as `HH:MM` (`8:05` becomes `08:05`), or `None` when it is not a
/// 24-hour time.
pub fn parse_time(text: &str) -> Option<String> {
    let (h, m) = text.trim().split_once(':')?;
    let ok =
        |s: &str, n: usize| (1..=n).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    if !ok(h, 2) || m.len() != 2 || !ok(m, 2) {
        return None;
    }
    let (h, m): (u8, u8) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

/// `words` plus `text` as one more keyword, or `None` when `text` is blank or
/// already listed (case does not matter).
pub fn add_keyword(words: &[String], text: &str) -> Option<Vec<String>> {
    let word = text.trim();
    let lower = word.to_lowercase();
    if word.is_empty() || words.iter().any(|w| w.trim().to_lowercase() == lower) {
        return None;
    }
    let mut next = words.to_vec();
    next.push(word.to_string());
    Some(next)
}

/// A notice above a page: the error ground with the message.
pub fn notice(
    id: impl Into<SharedString>,
    message: impl Into<SharedString>,
    soft: gpui::Hsla,
    colors: &ThemeColors,
) -> Div {
    let selector = id.into().to_string();
    div()
        .debug_selector(move || selector.clone())
        .flex()
        .items_center()
        .p(metrics::SPACE_5)
        .rounded(metrics::RADIUS_MD)
        .bg(soft)
        .border_1()
        .border_color(colors.border)
        .text_size(metrics::FONT_SIZE_SM)
        .text_color(colors.text)
        .child(message.into())
}

/// One end of a [`stepper`]: a square `−` or `+`. `name` is its accessible
/// name and tooltip ("Increase cache limit"). At the end of its range
/// (`enabled` false) it is muted, takes no focus and has no hover.
pub fn stepper_button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    name: impl Into<SharedString>,
    enabled: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let id = id.into();
    let name: SharedString = name.into();
    let el = if enabled {
        focusable(&id, colors)
            .text_color(colors.text)
            .hover(|h| h.bg(colors.element_hover))
    } else {
        let selector = id.to_string();
        div()
            .id(id.clone())
            .debug_selector(move || selector.clone())
            .text_color(colors.text_muted)
    };
    el.role(gpui::Role::Button)
        .aria_label(name.clone())
        .tooltip(crate::reactions::tooltip_shared(name))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(metrics::LAYOUT_ROW_COMPACT)
        .rounded(metrics::RADIUS_SM)
        .text_size(metrics::FONT_SIZE_MD)
        .child(label.into())
}

/// A number with its unit between a decrease and an increase button (the
/// boards' `.num`): `dec` and `inc` come from [`stepper_button`]. The number
/// is set in the mono face in a fixed minimum width so stacked steppers line up.
pub fn stepper(
    dec: Stateful<Div>,
    value: impl Into<SharedString>,
    unit: impl Into<SharedString>,
    inc: Stateful<Div>,
    colors: &ThemeColors,
    subtle: gpui::Hsla,
) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(metrics::SPACE_2)
        .p(metrics::SPACE_1)
        .rounded(metrics::RADIUS_MD)
        .border_1()
        .border_color(colors.border)
        .bg(colors.surface_background)
        .child(dec)
        .child(
            div()
                .flex()
                .items_baseline()
                .justify_center()
                .min_w(metrics::LAYOUT_STEPPER_VALUE)
                .gap(metrics::SPACE_2)
                .px(metrics::SPACE_2)
                .child(
                    div()
                        .font_family("JetBrains Mono")
                        .text_size(metrics::FONT_SIZE_XS)
                        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                        .text_color(colors.text)
                        .child(value.into()),
                )
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_XS)
                        .text_color(subtle)
                        .child(unit.into()),
                ),
        )
        .child(inc)
}

/// A small uppercase tag on a soft ground (the MCP tool list's READ and
/// WRITE).
pub fn tag(label: impl Into<SharedString>, soft: gpui::Hsla, text: gpui::Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(metrics::LAYOUT_STATUS_PILL)
        .px(metrics::SPACE_4)
        .rounded(metrics::RADIUS_FULL)
        .bg(soft)
        .text_color(text)
        .text_size(metrics::FONT_SIZE_2XS)
        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
        .child(label.into())
}

/// A status pill (the board's `.stat`): a dot in the text colour and the
/// label on the soft ground, 22 high.
pub fn status_pill(label: impl Into<SharedString>, soft: gpui::Hsla, text: gpui::Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(metrics::SPACE_3)
        .h(metrics::LAYOUT_STATUS_PILL)
        .px(metrics::SPACE_4)
        .rounded(metrics::RADIUS_FULL)
        .bg(soft)
        .text_color(text)
        .text_size(metrics::FONT_SIZE_2XS)
        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
        .child(
            div()
                .flex_none()
                .size(metrics::SPACE_3)
                .rounded(metrics::RADIUS_FULL)
                .bg(text),
        )
        .child(label.into())
}
