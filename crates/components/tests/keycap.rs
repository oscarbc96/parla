//! Key symbols draw as icons, never as text (the system fonts lack them), and the
//! rows that draw them speak their names.

use components::IconName;
use components::keycap::{
    Glyphs, Piece, RETURN, SHIFT_RETURN, glyph_icon, glyph_row, has_glyphs, pieces, spoken,
};
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    TestAppContext, Window, div,
};
use themes::metrics;

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
}

#[test]
fn modifiers_arrows_and_check_map_to_icons() {
    assert_eq!(glyph_icon('\u{21E7}'), Some(IconName::Shift));
    assert_eq!(glyph_icon('\u{2325}'), Some(IconName::OptionKey));
    assert_eq!(glyph_icon('\u{2303}'), Some(IconName::Control));
    assert_eq!(glyph_icon('\u{2318}'), Some(IconName::Command));
    assert_eq!(glyph_icon('\u{2713}'), Some(IconName::Check));
    assert_eq!(glyph_icon('\u{2714}'), Some(IconName::Check));
    assert_eq!(glyph_icon('\u{2191}'), Some(IconName::ArrowUp));
    assert_eq!(glyph_icon('\u{2193}'), Some(IconName::ArrowDown));
    assert_eq!(glyph_icon('\u{2190}'), Some(IconName::ArrowLeft));
    assert_eq!(glyph_icon('\u{2192}'), Some(IconName::ArrowRight));
    assert_eq!(glyph_icon('K'), None);
}

#[test]
fn no_text_piece_holds_a_symbol() {
    let all = "\u{2318}\u{21E7}\u{2325}\u{2303}\u{23CE}\u{232B}\u{21E5}\u{2713}\u{2191}K";
    assert!(has_glyphs(all));
    let p = pieces(all);
    assert_eq!(p.len(), 10);
    for piece in p {
        if let Piece::Text(t) = piece {
            assert!(t.chars().all(|c| glyph_icon(c).is_none()), "{t}");
        }
    }
    assert_eq!(pieces(SHIFT_RETURN).len(), 2);
    assert_eq!(
        pieces("\u{2318}K"),
        [Piece::Icon(IconName::Command), Piece::Text("K".into())]
    );
}

#[gpui::test]
fn symbols_are_spoken_by_name(cx: &mut TestAppContext) {
    init(cx);
    cx.update(|cx| {
        assert_eq!(
            spoken(&format!("{SHIFT_RETURN} new line"), cx),
            "Shift Return new line"
        );
        assert_eq!(spoken("\u{2318}\u{23CE} sends", cx), "Command Return sends");
        assert_eq!(spoken("Ctrl+\u{23CE}", cx), "Ctrl+ Return");
        assert_eq!(spoken("\u{2713}", cx), "Selected");
        assert_eq!(spoken("\u{2191}\u{2193}", cx), "Up arrow Down arrow");
        let g = Glyphs::new(format!("{RETURN} send"), cx);
        assert_eq!(g.label().map(|l| l.as_ref()), Some("Return send"));
        let plain = Glyphs::new("send", cx);
        assert!(plain.label().is_none());
        assert_eq!(plain.text().as_ref(), "send");
    });
}

struct Host;

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text: SharedString = format!("{SHIFT_RETURN} new line").into();
        div().child(
            glyph_row(&text, metrics::ICON_SIZE_SM, gpui::hsla(0., 0., 0., 1.), cx)
                .debug_selector(|| "keys".into()),
        )
    }
}

#[gpui::test]
fn a_glyph_row_draws_and_has_no_text_symbols(cx: &mut TestAppContext) {
    init(cx);
    let window = cx.add_window(|_, _| Host);
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    assert!(cx.debug_bounds("keys").is_some());
}
