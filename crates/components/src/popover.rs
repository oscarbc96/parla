//! A popover with Parla tokens, on top of the GPUI Base state and positioner.
//!
//! The Base popover owns the open lifecycle, focus capture and restoration,
//! Escape dismissal and deferred-popup registration. Base's `Popover` element
//! itself only offers corner anchoring, which does not flip, so this wrapper
//! positions its content with [`gpui_base::Positioner`] on the preferred side
//! and lets it flip and clamp at the window edge.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Context, ElementId, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Pixels, RenderOnce,
    Role, StatefulInteractiveElement as _, Styled as _, Window, canvas, deferred, div,
    prelude::FluentBuilder as _,
};
use gpui_base::{Align, Placement, PopoverState, Positioner};
use themes::{ActiveTheme as _, metrics};

const CONTEXT: &str = "Popover";
const CONTENT_CONTEXT: &str = "PopoverContent";

type OpenChange = Rc<dyn Fn(&bool, &mut Window, &mut App)>;
type TriggerBuilder = Box<dyn FnOnce(bool, &Window, &App) -> AnyElement>;
type ContentBuilder =
    Box<dyn FnOnce(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> AnyElement>;

#[derive(Default)]
struct TriggerBounds {
    bounds: Bounds<Pixels>,
    captured: bool,
}

/// A trigger and a token-styled popup on a side of it.
///
/// Uncontrolled unless [`Popover::open`] is set. The content takes the focus
/// when it opens and gives it back to the trigger when it closes.
#[derive(IntoElement)]
pub struct Popover {
    id: ElementId,
    placement: Placement,
    align: Align,
    offset: Pixels,
    width: Option<Pixels>,
    max_height: Option<Pixels>,
    open: Option<bool>,
    default_open: bool,
    trigger_focus: Option<FocusHandle>,
    trigger: Option<TriggerBuilder>,
    content: Option<ContentBuilder>,
    on_open_change: Option<OpenChange>,
}

impl Popover {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            placement: Placement::Bottom,
            align: Align::Center,
            offset: metrics::SPACE_0,
            width: None,
            max_height: None,
            open: None,
            default_open: false,
            trigger_focus: None,
            trigger: None,
            content: None,
            on_open_change: None,
        }
    }

    /// The side the content prefers; it flips when the side has no room.
    pub fn placement(mut self, placement: Placement) -> Self {
        self.placement = placement;
        self
    }

    /// Alignment along the chosen side.
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// Gap between the trigger and the content.
    pub fn offset(mut self, offset: Pixels) -> Self {
        self.offset = offset;
        self
    }

    pub fn width(mut self, width: Pixels) -> Self {
        self.width = Some(width);
        self
    }

    pub fn max_height(mut self, height: Pixels) -> Self {
        self.max_height = Some(height);
        self
    }

    /// Controls the open state; omit it for an uncontrolled popover.
    pub fn open(mut self, open: bool) -> Self {
        self.open = Some(open);
        self
    }

    pub fn default_open(mut self, open: bool) -> Self {
        self.default_open = open;
        self
    }

    /// The focus the content returns to on close. Defaults to the wrapper's
    /// own trigger handle when the caller has not supplied one.
    pub fn trigger_focus(mut self, focus: FocusHandle) -> Self {
        self.trigger_focus = Some(focus);
        self
    }

    pub fn trigger(mut self, trigger: impl IntoElement + 'static) -> Self {
        self.trigger = Some(Box::new(move |_, _, _| trigger.into_any_element()));
        self
    }

    /// A trigger built from the open state, for a selected or active look.
    pub fn trigger_with(
        mut self,
        trigger: impl FnOnce(bool, &Window, &App) -> AnyElement + 'static,
    ) -> Self {
        self.trigger = Some(Box::new(trigger));
        self
    }

    pub fn content<E>(
        mut self,
        content: impl FnOnce(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> E + 'static,
    ) -> Self
    where
        E: IntoElement,
    {
        self.content = Some(Box::new(move |state, window, cx| {
            content(state, window, cx).into_any_element()
        }));
        self
    }

    pub fn on_open_change(
        mut self,
        handler: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        let state = window.use_keyed_state(id.clone(), cx, |_, cx| {
            PopoverState::new(self.default_open, cx)
        });
        state.update(cx, |state, cx| {
            state.set_on_open_change(self.on_open_change.clone());
            if let Some(open) = self.open {
                state.sync_open(open, window, cx);
            }
        });

        let open = state.read(cx).is_open();
        let content_focus = state.read(cx).focus_handle(cx);
        let trigger_focus = match self.trigger_focus.clone() {
            Some(focus) => focus,
            None => window
                .use_keyed_state((id.clone(), "trigger-focus"), cx, |_, cx| {
                    cx.focus_handle().tab_stop(true)
                })
                .read(cx)
                .clone(),
        };
        let bounds = window.use_keyed_state((id.clone(), "trigger-bounds"), cx, |_, _| {
            TriggerBounds::default()
        });
        let Some(trigger) = self.trigger else {
            return div().into_any_element();
        };
        let trigger = (trigger)(open, window, cx);
        let parent_view = window.current_view();

        let toggle = {
            let state = state.clone();
            let trigger_focus = trigger_focus.clone();
            move |window: &mut Window, cx: &mut App| {
                cx.stop_propagation();
                trigger_focus.focus(window, cx);
                state.update(cx, |state, cx| state.toggle_open(window, cx));
                cx.notify(parent_view);
            }
        };
        let capture = bounds.clone();
        let canvas_state = state.clone();
        let root = div()
            .id(id)
            .relative()
            .key_context(CONTEXT)
            .on_action({
                let state = state.clone();
                move |_: &gpui_base::actions::Confirm, window, cx| {
                    state.update(cx, |state, cx| state.toggle_open(window, cx));
                    cx.notify(parent_view);
                }
            })
            .on_action(window.listener_for(&state, PopoverState::on_action_cancel))
            .child(
                div()
                    .track_focus(&trigger_focus)
                    .tab_stop(true)
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        toggle(window, cx);
                    })
                    .child(trigger)
                    .child(
                        canvas(
                            move |bounds, window, cx| {
                                let open = canvas_state.read(cx).is_open();
                                let first = capture.update(cx, |capture, _| {
                                    capture.bounds = bounds;
                                    let first = !capture.captured;
                                    capture.captured = true;
                                    first
                                });
                                if first && open {
                                    window.request_animation_frame();
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full()
                        .top_0()
                        .left_0(),
                    ),
            );

        if !open {
            return root.into_any_element();
        }
        let Some(content) = self.content else {
            return root.into_any_element();
        };
        let measured = bounds.read(cx);
        if !measured.captured {
            return root.into_any_element();
        }
        let trigger_bounds = measured.bounds;
        let colors = *cx.theme().colors();
        let body = state.update(cx, |state, cx| (content)(state, window, cx));
        let surface = div()
            .bg(colors.elevated_surface_background)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(metrics::RADIUS_LG)
            .p(metrics::SPACE_3)
            .when_some(self.width, |this, width| this.w(width))
            .when_some(self.max_height, |this, height| this.max_h(height))
            .child(body);
        let content = div()
            .id("content")
            .role(Role::Dialog)
            .occlude()
            .tab_group()
            .track_focus(&content_focus)
            .key_context(CONTENT_CONTEXT)
            .on_action(window.listener_for(&state, PopoverState::on_action_cancel))
            .on_mouse_down_out({
                let state = state.clone();
                move |_, window, cx| {
                    state.update(cx, |state, cx| state.dismiss(window, cx));
                    cx.notify(parent_view);
                }
            })
            .child(surface);
        let positioner = Positioner::side(trigger_bounds)
            .placement(self.placement)
            .align(self.align)
            .offset(self.offset)
            .occlude()
            .child(content);
        root.child(deferred(positioner)).into_any_element()
    }
}
