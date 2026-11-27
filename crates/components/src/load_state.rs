//! The two states of a pane that waits for the provider: a skeleton while the
//! first page is in flight (the States board's Loading frame) and a short
//! error with one Retry button when it failed. Presentation only: the host
//! pane owns the data and the retry.

use gpui::{
    AnyElement, App, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, div,
};
use themes::{ActiveTheme, metrics, parla_tokens};

use crate::controls::{ButtonKind, button, on_activate};

/// One placeholder bar, `width` a fraction of the line (percent).
fn bar(width: f32, height: gpui::Pixels, fill: gpui::Hsla) -> Div {
    div()
        .h(height)
        .w(gpui::relative(width / 100.0))
        .rounded(metrics::RADIUS_SM)
        .bg(fill)
}

/// Placeholder message rows (avatar and text bars), bottom aligned like the
/// real rows, with `label` under them. `rows` is how many to draw.
pub fn skeleton(key: &'static str, rows: usize, label: String, cx: &App) -> Div {
    let theme = cx.theme();
    let colors = theme.colors();
    let fill = colors.element_selected;
    // (name width, [line widths]) of each placeholder row, newest last.
    const SHAPES: [(f32, &[f32]); 3] = [(30.0, &[85.0, 60.0]), (22.0, &[70.0]), (26.0, &[90.0])];
    let mut col = div()
        .debug_selector(move || key.into())
        .flex()
        .flex_col()
        .justify_end()
        .size_full()
        .px(metrics::SPACE_7)
        .py(metrics::SPACE_6)
        // The board uses 18 px; 20 (SPACE_7) is the nearest scale step.
        .gap(metrics::SPACE_7);
    for (name, lines) in SHAPES.iter().take(rows.min(SHAPES.len())) {
        let mut text = div()
            .flex()
            .flex_col()
            .flex_1()
            .gap(metrics::SPACE_4)
            .child(bar(*name, metrics::SPACE_5, fill));
        for w in *lines {
            text = text.child(bar(*w, metrics::SPACE_5, fill));
        }
        col = col.child(
            div()
                .flex()
                .gap(metrics::SPACE_5)
                .child(
                    div()
                        .flex_none()
                        .size(metrics::LAYOUT_MSG_AVATAR)
                        .rounded(metrics::RADIUS_AVATAR)
                        .bg(fill),
                )
                .child(text),
        );
    }
    col.child(
        div()
            .text_size(metrics::FONT_SIZE_XS)
            .text_color(parla_tokens(theme).text_subtle)
            .child(label),
    )
}

/// What failed, one muted hint and a Retry button (a keyboard tab stop with
/// the focus ring) that runs `retry` on `host`.
pub fn load_error<T: 'static>(
    key: &'static str,
    id: &'static str,
    title: String,
    hint: String,
    retry_label: String,
    cx: &mut Context<T>,
    retry: impl Fn(&mut T, &mut gpui::Window, &mut Context<T>) + 'static,
) -> AnyElement {
    let colors = *cx.theme().colors();
    let btn = on_activate(
        crate::controls::compact(
            button(id, retry_label, ButtonKind::Primary, true, &colors)
                .debug_selector(move || id.into()),
        ),
        cx,
        retry,
    );
    crate::controls::empty_state(title, hint, &colors)
        .debug_selector(move || key.into())
        .size_full()
        .justify_center()
        .py_0()
        .gap(metrics::SPACE_4)
        .child(div().mt(metrics::SPACE_4).child(btn))
        .into_any_element()
}
