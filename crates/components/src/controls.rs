//! Shared controls (sign-in screens, composer): buttons, check boxes, field frames and
//! the keyboard focus ring. Presentation only; the screens own the behaviour.
//! Every colour comes from the active theme and every size from
//! `themes::metrics`.

use gpui::{
    BoxShadow, Div, Hsla as Colour, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement as _, Stateful, StatefulInteractiveElement, StyleRefinement, Styled, Window, div,
    point, prelude::*, px,
};
use themes::{ThemeColors, metrics};

/// How prominent a button is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// The one action the screen wants (filled).
    Primary,
    /// An alternative or a way back (outlined).
    Secondary,
}

/// The 2 px ring drawn inside a focused control's frame.
pub fn ring(colors: &ThemeColors) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: colors.border_focused,
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(0.0),
        spread_radius: metrics::SPACE_1,
        inset: true,
    }]
}

/// Style applied while a control has the keyboard focus: the frame border goes
/// transparent and the ring shows inside it (the board's `.input:focus`).
pub(crate) fn focus_style(colors: &ThemeColors) -> impl FnOnce(StyleRefinement) -> StyleRefinement {
    let colors = *colors;
    move |s| {
        s.border_color(gpui::transparent_black())
            .shadow(ring(&colors))
    }
}

/// A button that is a keyboard tab stop with the focus ring. A disabled one
/// is dimmed and neither takes focus nor clicks.
pub fn button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    kind: ButtonKind,
    enabled: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    button_frame(id, kind, enabled, colors).child(label.into())
}

/// A [`button`] without its label, for one that carries icons too.
pub fn button_frame(
    id: impl Into<gpui::ElementId>,
    kind: ButtonKind,
    enabled: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let (fill, border, text) = match kind {
        ButtonKind::Primary => (
            colors.element_background,
            colors.element_background,
            colors.text_on_accent,
        ),
        ButtonKind::Secondary => (colors.surface_background, colors.border, colors.text),
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .min_h(metrics::LAYOUT_INPUT_MIN)
        .px(metrics::SPACE_6)
        .py(metrics::SPACE_2)
        .rounded(metrics::RADIUS_MD)
        .border_1()
        .border_color(border)
        .bg(fill)
        .text_color(text)
        .text_size(metrics::FONT_SIZE_MD)
        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
        .when(enabled, |d| {
            d.cursor_pointer()
                .tab_stop(true)
                .focusable()
                .focus_visible(focus_style(colors))
        })
        .when(!enabled, |d| {
            // Half-strength colours, not element opacity: GPUI Fast applies
            // opacity to each paint, so a border the colour of the fill drew
            // as a darker outline and the label dimmed twice. A secondary
            // button dims its label like on main; a primary one keeps its
            // full-strength label on the dimmed fill.
            d.bg(dimmed(fill))
                .border_color(match kind {
                    ButtonKind::Primary => gpui::transparent_black(),
                    ButtonKind::Secondary => dimmed(border),
                })
                .when(kind == ButtonKind::Secondary, |d| {
                    d.text_color(dimmed(text))
                })
        })
}

/// A colour at half strength, for a disabled control's fill and border.
pub fn dimmed(c: Colour) -> Colour {
    c.opacity(0.5)
}

/// A filled button in the danger colour (`status.error`, text `danger_text`;
/// AA in both themes) for a step that cannot be undone, such as Archive.
pub fn danger_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    enabled: bool,
    theme: &themes::Theme,
) -> Stateful<Div> {
    let fill = theme.status().error;
    button(id, label, ButtonKind::Primary, enabled, theme.colors())
        .bg(if enabled { fill } else { dimmed(fill) })
        .border_color(if enabled {
            fill
        } else {
            gpui::transparent_black()
        })
        .text_color(themes::parla_tokens(theme).danger_text)
}

/// [`button`] in the small size (board `btn-sm`): 28pt high, XS type.
pub fn button_small(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    kind: ButtonKind,
    colors: &ThemeColors,
) -> Stateful<Div> {
    button(id, label, kind, true, colors)
        .min_h(metrics::LAYOUT_TITLEBAR_BUTTON)
        .px(metrics::SPACE_5)
        .py(metrics::SPACE_0)
        .rounded(metrics::RADIUS_SM)
        .text_size(metrics::FONT_SIZE_XS)
}

/// An icon-only button: a titlebar-button sized hit target with `icon` in muted
/// text, the hover ground and the focus ring, a tab stop that Enter and Space
/// activate. `label` is required: it names the control (tooltip) since the icon
/// carries no text. Callers add `.debug_selector(..)` and the like.
pub fn icon_button(
    id: impl Into<gpui::ElementId>,
    icon: crate::icon::IconName,
    label: impl Into<gpui::SharedString>,
    colors: &ThemeColors,
    on_activate: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    icon_button_sized(id, icon, label, IconButtonSize::Large, colors, on_activate)
}

/// Size of an [`icon_button`]: `Large` for panel and titlebar headers, `Small`
/// for controls inside a chip (a bookmark chip's remove).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconButtonSize {
    /// 18px glyph in a 28px target.
    Large,
    /// 14px glyph in a target shorter than a 26px chip.
    Small,
}

/// [`icon_button`] with an explicit [`IconButtonSize`].
pub fn icon_button_sized(
    id: impl Into<gpui::ElementId>,
    icon: crate::icon::IconName,
    label: impl Into<gpui::SharedString>,
    size: IconButtonSize,
    colors: &ThemeColors,
    on_activate: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    icon_button_styled(id, icon, label, size, None, false, colors, on_activate)
}

/// [`icon_button_sized`] with a glyph colour (muted text by default) and an
/// `active` state that keeps the selected ground, for a toggle such as the
/// header's details button.
#[allow(clippy::too_many_arguments)]
pub fn icon_button_styled(
    id: impl Into<gpui::ElementId>,
    icon: crate::icon::IconName,
    label: impl Into<gpui::SharedString>,
    size: IconButtonSize,
    glyph_color: Option<gpui::Hsla>,
    active: bool,
    colors: &ThemeColors,
    on_activate: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    let (target, glyph) = match size {
        IconButtonSize::Large => (metrics::LAYOUT_TITLEBAR_BUTTON, metrics::ICON_SIZE_LG),
        IconButtonSize::Small => (metrics::LAYOUT_ICON_BUTTON_SM, metrics::ICON_SIZE_SM),
    };
    let (hover, muted) = (colors.element_hover, colors.text_muted);
    let color = glyph_color.unwrap_or(muted);
    let label: gpui::SharedString = label.into();
    on_activate_app(
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(target)
            .rounded(metrics::RADIUS_MD)
            .cursor_pointer()
            .tab_stop(true)
            .focusable()
            .focus_visible(focus_style(colors))
            .when(active, |d| d.bg(colors.element_selected))
            .hover(move |d| d.bg(hover))
            .tooltip(crate::reactions::tooltip_shared(label))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .debug_selector(|| "icon-button-glyph".into())
                    .child(crate::icon::Icon::new(icon).size(glyph).color(color)),
            ),
        on_activate,
    )
}

/// Sizes a [`button`] like the composer Send: compact row height, small type.
pub fn compact(el: Stateful<Div>) -> Stateful<Div> {
    el.min_h(metrics::LAYOUT_ROW_COMPACT)
        .py_0()
        .px(metrics::SPACE_5)
        .text_size(metrics::FONT_SIZE_XS)
}

/// Keeps a mouse press on `el` from moving the keyboard focus to it, so
/// clicking a button next to a text field leaves the caret where it was. Tab
/// and Enter or Space still reach it.
pub(crate) fn no_focus_on_click(el: Stateful<Div>) -> Stateful<Div> {
    el.on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
        window.prevent_default();
    })
}

/// Runs `f` when `el` is clicked, or Enter or Space is pressed while it has the
/// focus.
pub(crate) fn on_activate<T: 'static>(
    el: Stateful<Div>,
    cx: &gpui::Context<T>,
    f: impl Fn(&mut T, &mut Window, &mut gpui::Context<T>) + 'static,
) -> Stateful<Div> {
    let f = std::rc::Rc::new(f);
    let on_key = f.clone();
    el.on_click(cx.listener({
        let f = f.clone();
        move |this, _, window, cx| f(this, window, cx)
    }))
    .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
        let k = &ev.keystroke;
        if k.modifiers.platform || k.modifiers.control || k.modifiers.alt {
            return;
        }
        if matches!(k.key.as_str(), "enter" | "space") {
            // Shift+Enter on a button neither activates it nor reaches a
            // parent's Enter handler; a held key does not repeat it.
            if !k.modifiers.shift && !ev.is_held {
                on_key(this, window, cx);
            }
            cx.stop_propagation();
        }
    }))
}

/// Like [`on_activate`] for a callback that needs no view: runs `f` on a click,
/// or on Enter or Space while focused (modifiers ignored, Shift+Enter swallowed).
pub fn on_activate_app(
    el: Stateful<Div>,
    f: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    let f = std::rc::Rc::new(f);
    let on_key = f.clone();
    el.on_click(move |_, window, cx| f(window, cx)).on_key_down(
        move |ev: &KeyDownEvent, window, cx| {
            let k = &ev.keystroke;
            if k.modifiers.platform || k.modifiers.control || k.modifiers.alt {
                return;
            }
            if matches!(k.key.as_str(), "enter" | "space") {
                if !k.modifiers.shift && !ev.is_held {
                    on_key(window, cx);
                }
                cx.stop_propagation();
            }
        },
    )
}

/// A check box: ticked shows a check mark on the accent fill. `border` is the
/// theme's `border_control` colour (3:1 on every ground) for the unticked box.
pub(crate) fn check_box(ticked: bool, colors: &ThemeColors, border: Colour) -> Div {
    let mark = if ticked {
        colors.element_background
    } else {
        colors.surface_background
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(metrics::SPACE_7)
        .rounded(metrics::RADIUS_SM)
        .border_1()
        .border_color(if ticked {
            colors.element_background
        } else {
            border
        })
        .bg(mark)
        .text_color(colors.text_on_accent)
        .text_size(metrics::FONT_SIZE_XS)
        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
        .children(ticked.then(|| {
            crate::icon::Icon::new(crate::icon::IconName::Check)
                .size(metrics::ICON_SIZE_SM)
                .color(colors.text_on_accent)
        }))
}

/// The frame around a text field; `focused` draws the focus ring. `border` is
/// the theme's `border_control` colour (3:1 on every ground).
pub(crate) fn field_frame(focused: bool, colors: &ThemeColors, border: Colour) -> Div {
    div()
        .flex()
        .items_center()
        .min_h(metrics::SPACE_10)
        .px(metrics::SPACE_5)
        .py(metrics::SPACE_2)
        .rounded(metrics::RADIUS_MD)
        .border_1()
        .border_color(border)
        .bg(colors.surface_background)
        .text_size(metrics::FONT_SIZE_MD)
        .when(focused, |d| {
            d.border_color(gpui::transparent_black())
                .shadow(ring(colors))
        })
}

/// An empty or error state: a centred title in strong text and one muted
/// line (the States board). Callers add an action as a child.
pub fn empty_state(title: String, hint: String, colors: &ThemeColors) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(metrics::SPACE_2)
        .py(metrics::SPACE_11)
        .child(
            div()
                .text_size(metrics::FONT_SIZE_MD)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(colors.text)
                .child(title),
        )
        .child(
            div()
                .text_size(metrics::FONT_SIZE_XS)
                .text_color(colors.text_muted)
                .child(hint),
        )
}

/// A card that groups text: surface fill, hairline border, large radius.
pub(crate) fn card(colors: &ThemeColors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(metrics::SPACE_4)
        .p(metrics::SPACE_6)
        .rounded(metrics::RADIUS_LG)
        .border_1()
        .border_color(colors.border)
        .bg(colors.surface_background)
}

/// A banner for a failed step, in the error colour.
pub(crate) fn error_banner(
    message: impl Into<gpui::SharedString>,
    error: Colour,
    colors: &ThemeColors,
) -> impl IntoElement {
    div()
        .flex()
        .gap(metrics::SPACE_4)
        .p(metrics::SPACE_5)
        .rounded(metrics::RADIUS_MD)
        .border_1()
        .border_color(error)
        .bg(colors.surface_background)
        .text_color(error)
        .text_size(metrics::FONT_SIZE_SM)
        .child(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use themes::ActiveTheme as _;

    fn label_color(el: Stateful<Div>) -> Option<Colour> {
        let mut el = el;
        el.style().text.color
    }

    fn colors(cx: &mut gpui::TestAppContext) -> ThemeColors {
        cx.update(themes::init_default);
        cx.update(|cx| *cx.theme().colors())
    }

    #[gpui::test]
    fn a_disabled_secondary_button_dims_its_label(cx: &mut gpui::TestAppContext) {
        let colors = colors(cx);
        assert_eq!(
            label_color(button("b", "Label", ButtonKind::Secondary, false, &colors)),
            Some(dimmed(colors.text))
        );
        // A primary button keeps its full-strength label on the dimmed fill.
        assert_eq!(
            label_color(button("b", "Label", ButtonKind::Primary, false, &colors)),
            Some(colors.text_on_accent)
        );
        assert_eq!(
            label_color(button("b", "Label", ButtonKind::Secondary, true, &colors)),
            Some(colors.text)
        );
    }
}
