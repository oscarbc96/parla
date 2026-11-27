//! Parla's dialogs on top of the GPUI Base modal host.
//!
//! Base owns the modal layer, focus trap, Escape and backdrop behaviour. This
//! wrapper supplies the Parla surface, title and description, and keeps Tab
//! cycling inside the dialog even when the window root is ours rather than
//! Base's `Root`.

use std::rc::Rc;

use gpui::{
    AnyElement, App, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Pixels, RenderOnce, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use themes::{ActiveTheme as _, metrics, parla_tokens};

/// The dialog's accent role: a normal dialog or an alert.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogKind {
    #[default]
    Dialog,
    Alert,
}

const CONTEXT: &str = "ParlaDialog";

type CloseHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// A modal dialog with a Parla surface and a focus trap.
#[derive(IntoElement)]
pub struct Dialog {
    focus: FocusHandle,
    kind: DialogKind,
    open: bool,
    title: Option<SharedString>,
    description: Option<SharedString>,
    label: Option<SharedString>,
    body: Option<AnyElement>,
    footer: Option<AnyElement>,
    width: Pixels,
    on_close: Option<CloseHandler>,
}

impl Dialog {
    pub fn new(cx: &mut App) -> Self {
        Self {
            focus: cx.focus_handle(),
            kind: DialogKind::Dialog,
            open: true,
            title: None,
            description: None,
            label: None,
            body: None,
            footer: None,
            width: metrics::LAYOUT_DIALOG_MD,
            on_close: None,
        }
    }

    /// An alert dialog: the alert role, and a backdrop press does not close it.
    pub fn alert(mut self) -> Self {
        self.kind = DialogKind::Alert;
        self
    }

    pub fn kind(mut self, kind: DialogKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The accessible name, when it differs from the visible title.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn body(mut self, body: impl IntoElement + 'static) -> Self {
        self.body = Some(body.into_any_element());
        self
    }

    pub fn footer(mut self, footer: impl IntoElement + 'static) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }

    pub fn width(mut self, width: Pixels) -> Self {
        self.width = width;
        self
    }

    /// Runs after the dialog closes (Escape, backdrop or a confirm action).
    pub fn on_close(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(handler));
        self
    }

    /// The surface's focus handle, for a caller that focuses it directly.
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }
}

impl RenderOnce for Dialog {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = *cx.theme().colors();
        let tokens = parla_tokens(cx.theme());
        let label = self.label.clone().or_else(|| self.title.clone());
        let focus = self.focus.clone();
        let next_focus = focus.clone();
        let previous_focus = focus.clone();
        let surface = div()
            .id("parla-dialog")
            .key_context(CONTEXT)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let key = &event.keystroke;
                if key.key != "tab"
                    || event.is_held
                    || key.modifiers.platform
                    || key.modifiers.control
                    || key.modifiers.alt
                {
                    return;
                }
                if key.modifiers.shift {
                    cycle_focus(window, cx, &previous_focus, false);
                } else {
                    cycle_focus(window, cx, &next_focus, true);
                }
                cx.stop_propagation();
            })
            .flex()
            .flex_col()
            .w(self.width)
            .bg(colors.elevated_surface_background)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(metrics::RADIUS_LG)
            .p(metrics::SPACE_8)
            .gap(metrics::SPACE_5)
            .when_some(self.title, |this, title| {
                this.child(
                    div()
                        .text_size(metrics::FONT_SIZE_XL)
                        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                        .child(title),
                )
            })
            .when_some(self.description, |this, description| {
                this.child(
                    div()
                        .text_size(metrics::FONT_SIZE_SM)
                        .text_color(colors.text_muted)
                        .child(description),
                )
            })
            .children(self.body)
            .children(self.footer);

        let backdrop = div()
            .absolute()
            .size_full()
            .bg(tokens.bg_overlay)
            .into_any_element();
        let surface = surface.into_any_element();
        let close = self.on_close.clone();
        let alert_close = self.on_close;
        match self.kind {
            DialogKind::Dialog => {
                let dialog = gpui_base::Dialog::new(cx)
                    .open(self.open)
                    .focus_handle(self.focus)
                    .backdrop(backdrop)
                    .popup(surface)
                    .on_close(move |_, window, cx| {
                        if let Some(handler) = &close {
                            handler(window, cx);
                        }
                    });
                let dialog = match label {
                    Some(label) => dialog.accessibility_label(label),
                    None => dialog,
                };
                dialog.into_any_element()
            }
            DialogKind::Alert => gpui_base::AlertDialog::new(cx)
                .open(self.open)
                .focus_handle(self.focus)
                .backdrop(backdrop)
                .popup(surface)
                .on_close(move |_, window, cx| {
                    if let Some(handler) = &alert_close {
                        handler(window, cx);
                    }
                })
                .into_any_element(),
        }
    }
}

/// Moves focus one stop and keeps it inside `container`: Base's Root does this
/// for its own root, and a Parla window root is not Base's.
fn cycle_focus(window: &mut Window, cx: &mut App, container: &FocusHandle, forward: bool) {
    let mut visited: Vec<Option<FocusHandle>> = Vec::new();
    loop {
        if forward {
            window.focus_next(cx);
        } else {
            window.focus_prev(cx);
        }
        if container.contains_focused(window, cx) {
            return;
        }
        let focused = window.focused(cx);
        if visited.contains(&focused) {
            // No reachable tab stop inside the trap: its own handle is safe.
            container.focus(window, cx);
            return;
        }
        visited.push(focused);
    }
}
