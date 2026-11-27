//! Page stacks with Parla tokens, on top of the GPUI Base navigation state.
//!
//! Base owns the history (push, pop, forward), the outgoing page's lifetime
//! and the transition clock. This wrapper supplies the timing from
//! [`themes::metrics`], switches at once under reduced motion, and draws the
//! two pages of a change as a horizontal slide.

use std::{rc::Rc, time::Duration};

use gpui::{
    AnyElement, AnyView, App, Entity, IntoElement, RenderOnce, StyleRefinement, Styled, Window, px,
};
use gpui_base::{
    NavMotion, NavOperation, NavPage, NavStackState, PresencePhase, StyledExt as _, Transition,
};
use themes::metrics;

use crate::Motion;

type ItemRenderer = Rc<dyn Fn(NavPage, &mut Window, &mut App) -> AnyElement>;

/// The motion a change runs with: immediate while Parla's motion is frozen
/// (the OS asks to reduce motion), animated otherwise.
pub fn motion(cx: &App) -> NavMotion {
    if crate::motion(cx) == Motion::Frozen {
        NavMotion::Immediate
    } else {
        NavMotion::Animated
    }
}

/// Pushes `view` on top of the stack, with Parla's motion.
pub fn push(state: &Entity<NavStackState>, view: impl Into<AnyView>, cx: &mut App) {
    let motion = motion(cx);
    state.update(cx, |state, cx| state.push(view, motion, cx));
}

/// Pops the top view, with Parla's motion. The root never pops.
pub fn pop(state: &Entity<NavStackState>, cx: &mut App) -> Option<AnyView> {
    let motion = motion(cx);
    state.update(cx, |state, cx| state.pop(motion, cx))
}

/// An unstyled host for a [`NavStackState`]: the current page fills the
/// container and a change runs over the stack's duration.
#[derive(IntoElement)]
pub struct NavStack {
    state: Entity<NavStackState>,
    duration: Duration,
    style: StyleRefinement,
    item: Option<ItemRenderer>,
}

impl NavStack {
    pub fn new(state: &Entity<NavStackState>) -> Self {
        Self {
            state: state.clone(),
            duration: metrics::DURATION_BASE,
            style: StyleRefinement::default(),
            item: None,
        }
    }

    /// The timing every push, pop and replace runs under.
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Renders each page instead of the default slide.
    pub fn item(
        mut self,
        render: impl Fn(NavPage, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.item = Some(Rc::new(render));
        self
    }
}

impl Styled for NavStack {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for NavStack {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let item: ItemRenderer = self
            .item
            .unwrap_or_else(|| Rc::new(default_item as fn(NavPage, &mut Window, &mut App) -> _));
        gpui_base::NavStack::new(&self.state)
            .transition(Transition::new(self.duration))
            .item(move |page, window, cx| item(page, window, cx))
            .refine_style(&self.style)
    }
}

/// The page leaving slides off to the side while the page arriving slides in
/// from the side the change came from, across the same progress.
fn default_item(page: NavPage, window: &mut Window, _: &mut App) -> AnyElement {
    let width = window.viewport_size().width;
    let progress = page.progress();
    let entering = page.phase() == PresencePhase::Entering;
    let offset = match page.operation() {
        Some(NavOperation::Push | NavOperation::Replace) if entering => width * (1.0 - progress),
        Some(NavOperation::Push | NavOperation::Replace) => -(width * progress),
        Some(NavOperation::Pop) if entering => -(width * (1.0 - progress)),
        Some(NavOperation::Pop) => width * progress,
        _ => px(0.0),
    };
    page.left(offset).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn frozen_motion_is_immediate(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::set_motion(cx, Motion::Frozen);
            assert_eq!(motion(cx), NavMotion::Immediate);
            crate::set_motion(cx, Motion::Live);
            assert_eq!(motion(cx), NavMotion::Animated);
        });
    }
}
