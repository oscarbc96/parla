//! The theme-coloured logo: token substitution, one-entry cache, rendering.

use std::sync::Arc;

use components::logo::{logo, logo_image, logo_svg};
use gpui::{Context, IntoElement, Render, TestAppContext, Window, px};
use themes::{Appearance, LogoTokens, color};

fn tokens() -> LogoTokens {
    LogoTokens::defaults(Appearance::Dark)
}

#[test]
fn substitution_writes_every_token() {
    let mut t = tokens();
    t.bubble = color::rgb_to_hsla(gpui::rgb(0x010203));
    t.rim = color::rgb_to_hsla(gpui::rgb(0x040506));
    t.accent_1 = color::rgb_to_hsla(gpui::rgb(0x0a0b0c));
    t.accent_2 = color::rgb_to_hsla(gpui::rgb(0x0d0e0f));
    t.accent_3 = color::rgb_to_hsla(gpui::rgb(0x101112));
    let svg = logo_svg(&t);
    let bubble = svg
        .lines()
        .find(|l| l.contains("class=\"bubble\""))
        .unwrap();
    assert!(bubble.contains("fill=\"#010203\""), "{bubble:.120}");
    assert!(bubble.contains("stroke=\"#040506\""));
    for (class, hex) in [
        ("accent-1", "#0a0b0c"),
        ("accent-2", "#0d0e0f"),
        ("accent-3", "#101112"),
    ] {
        let line = svg
            .lines()
            .find(|l| l.contains(&format!("class=\"{class}\"")))
            .unwrap();
        assert!(line.contains(&format!("fill=\"{hex}\"")), "{class}");
    }
    assert!(!svg.contains("#FFA611"));
}

#[test]
fn transparent_rim_means_no_stroke() {
    let svg = logo_svg(&LogoTokens::defaults(Appearance::Light));
    let bubble = svg
        .lines()
        .find(|l| l.contains("class=\"bubble\""))
        .unwrap();
    assert!(bubble.contains("stroke=\"none\""));
}

#[test]
fn svg_is_rasterised_small() {
    let svg = logo_svg(&tokens());
    assert!(svg.contains("width=\"128\""));
    assert!(!svg.contains("width=\"1024\""));
}

#[gpui::test]
fn cache_keeps_one_entry(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let a = logo_image(&tokens(), cx);
        let b = logo_image(&tokens(), cx);
        assert!(Arc::ptr_eq(&a, &b));
        let c = logo_image(&LogoTokens::defaults(Appearance::Light), cx);
        assert!(!Arc::ptr_eq(&a, &c));
        let d = logo_image(&LogoTokens::defaults(Appearance::Light), cx);
        assert!(Arc::ptr_eq(&c, &d));
    });
}

#[gpui::test]
fn image_rasterises_with_colour(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let image = logo_image(&tokens(), cx);
        let data = image.to_image_data(cx.svg_renderer()).unwrap();
        let size = data.size(0);
        assert_eq!((size.width.0, size.height.0), (256, 256));
    });
}

struct Host;

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        logo(px(64.0), cx)
    }
}

#[gpui::test]
fn logo_renders_in_a_window(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (_view, cx) = cx.add_window_view(|_, _| Host);
    cx.run_until_parked();
    let bounds = cx.debug_bounds("logo").expect("the logo is drawn");
    assert_eq!(
        bounds.size,
        gpui::size(gpui::px(64.0), gpui::px(64.0)),
        "the logo draws at the size it was asked for"
    );
}
