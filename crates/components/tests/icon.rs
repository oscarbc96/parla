//! `Icon` and `icon_button`: size from the icon scale, click and keyboard
//! activation, and the tab stop.

use components::{Icon, IconName, controls};
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div,
};
use themes::{ActiveTheme as _, metrics};

struct Host {
    pressed: u32,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        let this = cx.entity();
        div()
            .flex()
            .items_start()
            .gap(metrics::SPACE_4)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .debug_selector(|| "icon-wrap".into())
                    .child(Icon::new(IconName::Search).size(metrics::ICON_SIZE_SM)),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .debug_selector(|| "icon-default".into())
                    .child(Icon::new(IconName::Bell)),
            )
            .child(
                controls::icon_button("close", IconName::X, "Close", &colors, move |_, cx| {
                    this.update(cx, |h, _| h.pressed += 1)
                })
                .debug_selector(|| "close".into()),
            )
    }
}

fn host(cx: &mut TestAppContext) -> (gpui::Entity<Host>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    cx.add_window_view(|_, _| Host { pressed: 0 })
}

#[gpui::test]
fn icon_uses_the_scale_sizes(cx: &mut TestAppContext) {
    let (_h, vcx) = host(cx);
    let small = vcx.debug_bounds("icon-wrap").expect("drawn");
    assert_eq!(small.size.width, metrics::ICON_SIZE_SM);
    assert_eq!(small.size.height, metrics::ICON_SIZE_SM);
    let dflt = vcx.debug_bounds("icon-default").expect("drawn");
    assert_eq!(dflt.size.width, metrics::ICON_SIZE_MD);
}

#[gpui::test]
fn icon_button_activates_on_click(cx: &mut TestAppContext) {
    let (h, vcx) = host(cx);
    let b = vcx.debug_bounds("close").expect("drawn");
    assert_eq!(b.size.width, metrics::LAYOUT_TITLEBAR_BUTTON);
    vcx.simulate_click(b.center(), gpui::Modifiers::default());
    assert_eq!(h.read_with(vcx, |h, _| h.pressed), 1);
}

#[gpui::test]
fn icon_button_is_a_tab_stop_and_activates_with_enter_and_space(cx: &mut TestAppContext) {
    let (h, vcx) = host(cx);
    // Tab order is the window's: the button must be a stop in it.
    vcx.update(|window, cx| window.focus_next(cx));
    vcx.simulate_keystrokes("enter");
    assert_eq!(h.read_with(vcx, |h, _| h.pressed), 1, "enter");
    vcx.simulate_keystrokes("space");
    assert_eq!(h.read_with(vcx, |h, _| h.pressed), 2, "space");
}

struct SmallHost;

impl Render for SmallHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        div().flex().items_start().child(
            controls::icon_button_sized(
                "chip-x",
                IconName::X,
                "Remove",
                controls::IconButtonSize::Small,
                &colors,
                |_, _| {},
            )
            .debug_selector(|| "chip-x".into()),
        )
    }
}

#[gpui::test]
fn small_icon_button_has_a_small_glyph_and_fits_a_chip(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (_h, vcx) = cx.add_window_view(|_, _| SmallHost);
    let target = vcx.debug_bounds("chip-x").expect("drawn");
    assert_eq!(target.size.width, metrics::LAYOUT_ICON_BUTTON_SM);
    assert_eq!(target.size.height, metrics::LAYOUT_ICON_BUTTON_SM);
    assert!(
        target.size.height < metrics::LAYOUT_AVATAR_SM,
        "inside a 26px chip"
    );
    let glyph = vcx.debug_bounds("icon-button-glyph").expect("glyph drawn");
    assert_eq!(glyph.size.width, metrics::ICON_SIZE_SM);
    assert_eq!(glyph.size.height, metrics::ICON_SIZE_SM);
}
