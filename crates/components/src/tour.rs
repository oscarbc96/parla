//! First-run tour (board FirstRunTour): coach marks over the main window,
//! each with a spotlight on the part it explains. The host shows it once,
//! after the first sync, and remembers when it was skipped or finished.

use std::{cell::Cell, rc::Rc};

use gpui::{
    App, Bounds, Context, Div, EntityId, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, Pixels, Render, Window, div, prelude::*,
};
use themes::{ActiveTheme, metrics, parla_tokens};

use crate::controls::{ButtonKind, button_small, icon_button, on_activate};

/// Where the parts the tips point at sit in the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TourLayout {
    /// Height of the titlebar above the rail and sidebar.
    pub top: Pixels,
    pub rail: Pixels,
    pub sidebar: Pixels,
}

impl Default for TourLayout {
    fn default() -> Self {
        Self {
            top: metrics::SPACE_0,
            rail: metrics::LAYOUT_RAIL,
            sidebar: metrics::LAYOUT_SIDEBAR,
        }
    }
}

/// The painted bounds of the parts the tips point at. The host wraps each part
/// with [`TourTargets::track`]; the tour spotlights those bounds, so banners and
/// status lines below the columns are never covered.
#[derive(Clone, Default)]
pub struct TourTargets {
    sidebar: Rc<Cell<Option<Bounds<Pixels>>>>,
    rail: Rc<Cell<Option<Bounds<Pixels>>>>,
    tour: Rc<Cell<Option<EntityId>>>,
}

impl gpui::Global for TourTargets {}

/// Which part a [`TourTargets::track`] call measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TourPart {
    Sidebar,
    Rail,
}

impl TourTargets {
    fn cell(&self, part: TourPart) -> &Rc<Cell<Option<Bounds<Pixels>>>> {
        match part {
            TourPart::Sidebar => &self.sidebar,
            TourPart::Rail => &self.rail,
        }
    }

    /// Forgets the tour and the measured bounds (the tour was dismissed).
    pub fn clear(&self) {
        self.tour.set(None);
        self.sidebar.set(None);
        self.rail.set(None);
    }

    /// The last painted bounds of `part`.
    pub fn bounds(&self, part: TourPart) -> Option<Bounds<Pixels>> {
        self.cell(part).get()
    }

    /// Records the painted bounds of `el` as `part`, and redraws the tour when
    /// they change (no timer: only a layout change repaints it).
    pub fn track(&self, part: TourPart, el: impl IntoElement) -> Div {
        let cell = self.cell(part).clone();
        let tour = self.tour.clone();
        // A wrapper sized by `el`, so its bounds include `el`'s own border.
        div().relative().h_full().child(el).child(
            gpui::canvas(
                |_, _, _| (),
                move |bounds, _, _, cx| {
                    if cell.replace(Some(bounds)) != Some(bounds)
                        && let Some(id) = tour.get()
                    {
                        cx.notify(id);
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
    }
}

/// The shortcuts the tips name, as the keymap binds them ("" when unbound).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TourKeys {
    pub workspace_first: String,
    pub workspace_last: String,
    pub switcher: String,
    pub shortcuts: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TourEvent {
    /// Skipped, closed or finished: the host forgets the tour for good.
    Dismissed,
}

/// The parts the tips point at, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Sidebar,
    Rail,
    /// No part of the window: the card sits in the middle.
    None,
}

const TIPS: [Target; 4] = [Target::Sidebar, Target::Rail, Target::None, Target::None];

pub struct Tour {
    tip: usize,
    layout: TourLayout,
    keys: TourKeys,
    targets: TourTargets,
    focus: FocusHandle,
}

impl EventEmitter<TourEvent> for Tour {}

impl Tour {
    pub fn new(layout: TourLayout, cx: &mut Context<Self>) -> Self {
        Self {
            tip: 0,
            layout,
            keys: TourKeys::default(),
            targets: TourTargets::default(),
            focus: cx.focus_handle(),
        }
    }

    pub fn with_keys(mut self, keys: TourKeys) -> Self {
        self.keys = keys;
        self
    }

    /// Spotlights the host's measured bounds instead of the layout numbers.
    pub fn with_targets(mut self, targets: TourTargets, cx: &Context<Self>) -> Self {
        targets.tour.set(Some(cx.entity_id()));
        self.targets = targets;
        self
    }

    /// The spotlight rectangle of the tip shown: x, y, width, height.
    pub fn spotlight(&self, view_height: Pixels) -> Option<(Pixels, Pixels, Pixels, Pixels)> {
        let l = self.layout;
        let measured = |part| {
            self.targets
                .bounds(part)
                .map(|b| (b.origin.x, b.origin.y, b.size.width, b.size.height))
        };
        match TIPS[self.tip] {
            Target::Sidebar => measured(TourPart::Sidebar).or(Some((
                l.rail,
                l.top,
                l.sidebar,
                view_height - l.top,
            ))),
            Target::Rail => measured(TourPart::Rail).or(Some((
                metrics::SPACE_0,
                l.top,
                l.rail,
                view_height - l.top,
            ))),
            Target::None => None,
        }
    }

    pub fn focus_handle_for(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// The tip shown, from 0.
    pub fn tip(&self) -> usize {
        self.tip
    }

    pub fn tips(&self) -> usize {
        TIPS.len()
    }

    /// The next tip; after the last one the tour ends.
    pub fn next(&mut self, cx: &mut Context<Self>) {
        if self.tip + 1 >= TIPS.len() {
            return self.skip(cx);
        }
        self.tip += 1;
        cx.notify();
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        if self.tip > 0 {
            self.tip -= 1;
            cx.notify();
        }
    }

    /// Ends the tour (Skip tour, the close button, Escape, the last Done).
    pub fn skip(&mut self, cx: &mut Context<Self>) {
        cx.emit(TourEvent::Dismissed);
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if m.platform || m.control || m.alt || m.shift {
            return;
        }
        match k.key.as_str() {
            "escape" => self.skip(cx),
            "enter" | "right" => self.next(cx),
            "left" => self.back(cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Title and body of the current tip.
    fn text(&self, cx: &App) -> (String, String) {
        // The catalog's own word for "no shortcut", which its selectors match.
        let unbound = t!(cx, "tour-keys-unbound").into_owned();
        let none = |k: &str| {
            if k.is_empty() {
                unbound.clone()
            } else {
                k.to_string()
            }
        };
        let k = &self.keys;
        match self.tip {
            0 => (
                t!(cx, "tour-sidebar-title").into_owned(),
                t!(cx, "tour-sidebar-body").into_owned(),
            ),
            1 => (
                t!(cx, "tour-rail-title").into_owned(),
                t!(
                    cx,
                    "tour-rail-body",
                    first = none(&k.workspace_first),
                    last = none(&k.workspace_last)
                )
                .into_owned(),
            ),
            2 => (
                t!(cx, "tour-jump-title").into_owned(),
                t!(cx, "tour-jump-body", keys = none(&k.switcher)).into_owned(),
            ),
            _ => (
                t!(cx, "tour-keys-title").into_owned(),
                t!(cx, "tour-keys-body", keys = none(&k.shortcuts)).into_owned(),
            ),
        }
    }

    fn coach(&self, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let colors = *cx.theme().colors();
        let tokens = parla_tokens(cx.theme());
        let (title, body) = self.text(cx);
        let last = self.tip + 1 == TIPS.len();
        let mut dots = div().flex().items_center().gap(metrics::SPACE_3);
        for i in 0..TIPS.len() {
            let on = i == self.tip;
            dots = dots.child(
                div()
                    .h(metrics::SPACE_3)
                    .w(if on {
                        metrics::SPACE_6
                    } else {
                        metrics::SPACE_3
                    })
                    .rounded_full()
                    .bg(if on {
                        tokens.signal
                    } else {
                        colors.border_strong
                    }),
            );
        }
        let this = cx.entity().downgrade();
        let close = icon_button(
            "tour-close",
            crate::icon::IconName::X,
            t!(cx, "tour-close").into_owned(),
            &colors,
            move |_, cx| {
                this.update(cx, |t, cx| t.skip(cx)).ok();
            },
        )
        .debug_selector(|| "tour-close".into());
        let skip = on_activate(
            button_small(
                "tour-skip",
                t!(cx, "tour-skip").into_owned(),
                ButtonKind::Secondary,
                &colors,
            )
            .border_color(gpui::transparent_black())
            .bg(gpui::transparent_black())
            .text_color(colors.text_muted)
            .debug_selector(|| "tour-skip".into()),
            cx,
            |this, _, cx| this.skip(cx),
        );
        let back = (self.tip > 0).then(|| {
            on_activate(
                button_small(
                    "tour-back",
                    t!(cx, "tour-back").into_owned(),
                    ButtonKind::Secondary,
                    &colors,
                )
                .debug_selector(|| "tour-back".into()),
                cx,
                |this, window, cx| {
                    window.focus(&this.focus, cx);
                    this.back(cx)
                },
            )
        });
        let next = on_activate(
            button_small(
                "tour-next",
                if last {
                    t!(cx, "tour-done").into_owned()
                } else {
                    t!(cx, "tour-next").into_owned()
                },
                ButtonKind::Primary,
                &colors,
            )
            .debug_selector(|| "tour-next".into()),
            cx,
            |this, window, cx| {
                window.focus(&this.focus, cx);
                this.next(cx)
            },
        );
        div()
            .id("tour-coach")
            .debug_selector(|| "tour-coach".into())
            // Only the card holds the tour's focus (Escape, arrows); the rest
            // of the window stays the app's.
            .key_context("Tour")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .flex_col()
            .gap(metrics::SPACE_3)
            .w(metrics::LAYOUT_COACH_WIDTH)
            .p(metrics::SPACE_7)
            .pb(metrics::SPACE_6)
            .rounded(metrics::RADIUS_XL)
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .text_color(colors.text)
            .shadow_lg()
            .occlude()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_4)
                    .child(
                        div()
                            .px(metrics::SPACE_4)
                            .py(metrics::SPACE_1)
                            .rounded_full()
                            .bg(tokens.signal_soft)
                            .text_color(tokens.signal_text)
                            .text_size(metrics::FONT_SIZE_XS)
                            .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                            .child(
                                t!(cx, "tour-tip", tip = self.tip + 1, total = TIPS.len())
                                    .into_owned(),
                            ),
                    )
                    .child(div().flex_1())
                    .child(close),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_LG)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_SM)
                    .text_color(colors.text_muted)
                    .child(
                        crate::keycap::glyph_row(
                            &body.into(),
                            metrics::ICON_SIZE_SM,
                            colors.text_muted,
                            cx,
                        )
                        .flex_wrap(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_4)
                    .mt(metrics::SPACE_5)
                    .child(dots)
                    .child(div().flex_1())
                    .child(skip)
                    .children(back)
                    .child(next),
            )
    }
}

/// Keeps a card of `size` starting at `at` inside `0..limit`, with a margin.
fn clamp_axis(at: Pixels, size: Pixels, limit: Pixels) -> Pixels {
    let max = (limit - size - metrics::SPACE_6).max(metrics::SPACE_6);
    at.min(max).max(metrics::SPACE_0)
}

impl Focusable for Tour {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Tour {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = parla_tokens(cx.theme());
        let view = window.viewport_size();
        let spot = self.spotlight(view.height);
        let scrim = |x: Pixels, y: Pixels, w: Pixels, h: Pixels| {
            div()
                .absolute()
                .left(x)
                .top(y)
                .w(w.max(metrics::SPACE_0))
                .h(h.max(metrics::SPACE_0))
                .bg(tokens.bg_overlay)
        };
        let mut root = div()
            .id("tour")
            .debug_selector(|| "tour".into())
            .absolute()
            // A hint card, not a modal: clicks outside the card reach the app
            // underneath, so the user can click into an input and type.
            .inset_0();
        let coach = self.coach(cx);
        match spot {
            Some((x, y, w, h)) => {
                let right = x + w;
                root = root
                    .child(scrim(metrics::SPACE_0, metrics::SPACE_0, view.width, y))
                    .child(scrim(metrics::SPACE_0, y, x, h))
                    .child(scrim(right, y, view.width - right, h))
                    .child(scrim(
                        metrics::SPACE_0,
                        y + h,
                        view.width,
                        view.height - y - h,
                    ))
                    .child(
                        div()
                            .debug_selector(|| "tour-spotlight".into())
                            .absolute()
                            .left(x)
                            .top(y)
                            .w(w)
                            .h(h)
                            .border_2()
                            .border_color(tokens.signal),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(clamp_axis(
                                right + metrics::SPACE_6,
                                metrics::LAYOUT_COACH_WIDTH,
                                view.width,
                            ))
                            .top(clamp_axis(
                                y + metrics::SPACE_9,
                                metrics::LAYOUT_COACH_HEIGHT,
                                view.height,
                            ))
                            .child(coach),
                    );
            }
            None => {
                root = root.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(tokens.bg_overlay)
                        .child(coach),
                );
            }
        }
        root
    }
}
