//! The Settings controls: switch, segmented control, choice card and select
//! react to a click and to Enter or Space on the focused control.

use components::controls::on_activate_app;
use components::settings_controls::{
    ThemePreview, add_keyword, choice_card, density_card, keyword_chip, keyword_field, notice,
    parse_time, radio_card, segment, segmented, select, select_option, select_trigger, switch,
    theme_card, time_field,
};
use components::text_input::TextInput;
use gpui::{
    Context, Entity, IntoElement, Modifiers, Render, TestAppContext, VisualTestContext, Window,
    div, prelude::*,
};
use themes::ActiveTheme as _;

struct Host {
    on: bool,
    picked: usize,
    card: bool,
    open: bool,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        let this = cx.entity();
        let toggle = {
            let this = this.clone();
            on_activate_app(switch("sw", self.on, &colors), move |_, cx| {
                this.update(cx, |h, cx| {
                    h.on = !h.on;
                    cx.notify();
                })
            })
        };
        let segs = (0..3).map(|i| {
            let this = this.clone();
            on_activate_app(
                segment(
                    format!("seg-{i}"),
                    format!("S{i}"),
                    self.picked == i,
                    &colors,
                ),
                move |_, cx| {
                    this.update(cx, |h, cx| {
                        h.picked = i;
                        cx.notify();
                    })
                },
            )
        });
        let card = {
            let this = this.clone();
            on_activate_app(
                choice_card(
                    "card",
                    "Title",
                    "Body",
                    self.card,
                    &colors,
                    colors.text_muted,
                ),
                move |_, cx| {
                    this.update(cx, |h, cx| {
                        h.card = true;
                        cx.notify();
                    })
                },
            )
        };
        let select = {
            let this = this.clone();
            on_activate_app(
                select_trigger("sel", "System", self.open, &colors),
                move |_, cx| {
                    this.update(cx, |h, cx| {
                        h.open = !h.open;
                        cx.notify();
                    })
                },
            )
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(toggle)
            .child(segmented(segs, &colors))
            .child(card)
            .child(select)
            .children(
                self.open
                    .then(|| select_option("opt", "English", false, &colors)),
            )
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<Host>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    cx.add_window_view(|_, _| Host {
        on: false,
        picked: 0,
        card: false,
        open: false,
    })
}

/// Moves the focus along the tab stops `n` times (the window's own Tab key).
fn next(cx: &mut VisualTestContext, n: usize) {
    for _ in 0..n {
        cx.update(|w, cx| w.focus_next(cx));
    }
}

fn click(cx: &mut VisualTestContext, id: &'static str) {
    let b = cx.debug_bounds(id).unwrap_or_else(|| panic!("{id} drawn"));
    cx.simulate_click(b.center(), Modifiers::none());
}

#[gpui::test]
fn switch_toggles_on_click(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    click(cx, "sw");
    assert!(host.read_with(cx, |h, _| h.on));
    click(cx, "sw");
    assert!(!host.read_with(cx, |h, _| h.on));
}

#[gpui::test]
fn switch_toggles_with_keyboard(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    next(cx, 1);
    cx.simulate_keystrokes("enter");
    assert!(
        host.read_with(cx, |h, _| h.on),
        "enter on the first tab stop"
    );
    cx.simulate_keystrokes("space");
    assert!(!host.read_with(cx, |h, _| h.on));
}

#[gpui::test]
fn segmented_picks_the_clicked_segment(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    click(cx, "seg-2");
    assert_eq!(host.read_with(cx, |h, _| h.picked), 2);
    click(cx, "seg-1");
    assert_eq!(host.read_with(cx, |h, _| h.picked), 1);
}

#[gpui::test]
fn card_selects_and_select_opens(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    click(cx, "card");
    assert!(host.read_with(cx, |h, _| h.card));
    assert!(cx.debug_bounds("opt").is_none());
    click(cx, "sel");
    assert!(host.read_with(cx, |h, _| h.open));
    assert!(cx.debug_bounds("opt").is_some(), "the list is shown");
}

#[gpui::test]
fn every_control_is_a_tab_stop(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    // switch, 3 segments, card, select: tab through them and pick the last.
    next(cx, 6);
    cx.simulate_keystrokes("enter");
    assert!(
        host.read_with(cx, |h, _| h.open),
        "sixth stop is the select"
    );
}

#[test]
fn parse_time_accepts_24_hour_times_and_pads_them() {
    assert_eq!(parse_time("20:00").as_deref(), Some("20:00"));
    assert_eq!(parse_time(" 8:05 ").as_deref(), Some("08:05"));
    assert_eq!(parse_time("00:00").as_deref(), Some("00:00"));
    assert_eq!(parse_time("23:59").as_deref(), Some("23:59"));
    for bad in [
        "", "24:00", "12:60", "12:5", "1200", "ab:cd", "1:2:3", "-1:00", "12:",
    ] {
        assert_eq!(parse_time(bad), None, "{bad:?}");
    }
}

#[test]
fn add_keyword_trims_and_refuses_blank_and_duplicates() {
    let words = vec!["Release".to_string()];
    assert_eq!(
        add_keyword(&words, "  on-call "),
        Some(vec!["Release".into(), "on-call".into()])
    );
    assert_eq!(add_keyword(&words, "   "), None);
    assert_eq!(add_keyword(&words, "release"), None, "case is ignored");
    assert_eq!(add_keyword(&words, "RELEASE"), None, "lowercase compare");
    let padded = vec![" Straße ".to_string()];
    assert_eq!(
        add_keyword(&padded, "straße"),
        None,
        "existing entries are trimmed"
    );
    assert_eq!(add_keyword(&[], "design review").unwrap().len(), 1);
}

struct Cards {
    picked: &'static str,
    removed: Vec<usize>,
    words: Vec<String>,
    input: Entity<TextInput>,
    time: Entity<TextInput>,
}

impl Render for Cards {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        let this = cx.entity();
        let preview = ThemePreview {
            background: colors.background,
            sidebar: colors.panel_background,
            sidebar_text: colors.text_muted,
            text: colors.text,
            accent: colors.element_background,
        };
        let pick = |id: &'static str, el: gpui::Stateful<gpui::Div>| {
            let this = this.clone();
            on_activate_app(el, move |_, cx| {
                this.update(cx, |h, cx| {
                    h.picked = id;
                    cx.notify();
                })
            })
        };
        let chips: Vec<_> = self
            .words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let this = this.clone();
                on_activate_app(
                    keyword_chip(format!("kw-{i}"), w, &colors, colors.text),
                    move |_, cx| {
                        this.update(cx, |h, cx| {
                            h.removed.push(i);
                            cx.notify();
                        })
                    },
                )
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(pick(
                "theme",
                theme_card(
                    "theme",
                    "Parla Light",
                    "Light",
                    self.picked == "theme",
                    preview,
                    &colors,
                    colors.text_muted,
                ),
            ))
            .child(pick(
                "dens",
                density_card("dens", "Cozy", 2, true, self.picked == "dens", &colors),
            ))
            .child(pick(
                "radio",
                radio_card(
                    "radio",
                    "Nothing",
                    "Badges only",
                    self.picked == "radio",
                    &colors,
                    colors.text_muted,
                ),
            ))
            .child(keyword_field(
                chips,
                self.input.clone(),
                false,
                &colors,
                colors.border_strong,
            ))
            .child(time_field(
                "time",
                self.time.clone(),
                false,
                &colors,
                colors.border_strong,
            ))
            .child(notice(
                "note",
                "Could not save",
                colors.surface_background,
                &colors,
            ))
    }
}

fn cards(cx: &mut TestAppContext) -> (Entity<Cards>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    cx.add_window_view(|_, cx| Cards {
        picked: "",
        removed: Vec::new(),
        words: vec!["release".into(), "on-call".into()],
        input: cx.new(TextInput::new),
        time: cx.new(TextInput::new),
    })
}

#[gpui::test]
fn theme_density_and_radio_cards_activate(cx: &mut TestAppContext) {
    let (host, cx) = cards(cx);
    for id in ["theme", "dens", "radio"] {
        click(cx, id);
        assert_eq!(host.read_with(cx, |h, _| h.picked), id);
    }
    assert!(cx.debug_bounds("note").is_some(), "the notice is drawn");
}

#[gpui::test]
fn keyword_chip_removes_by_click_and_keyboard(cx: &mut TestAppContext) {
    let (host, cx) = cards(cx);
    click(cx, "kw-1");
    assert_eq!(host.read_with(cx, |h, _| h.removed.clone()), [1]);
    // The chips are the first tab stops here (after theme, density, radio).
    next(cx, 4);
    cx.simulate_keystrokes("enter");
    assert_eq!(host.read_with(cx, |h, _| h.removed.clone()), [1, 0]);
}

#[gpui::test]
fn keyword_and_time_fields_take_typed_text(cx: &mut TestAppContext) {
    let (host, cx) = cards(cx);
    assert!(cx.debug_bounds("settings-keyword-input").is_some());
    click(cx, "time");
    cx.simulate_input("8:30");
    let time = host.read_with(cx, |h, _| h.time.clone());
    assert_eq!(time.read_with(cx, |t, _| t.text().to_string()), "8:30");
    assert_eq!(parse_time("8:30").as_deref(), Some("08:30"));
}

/// A select alone in the window, at its top or its bottom edge.
struct Edge {
    bottom: bool,
    open: bool,
}

impl Render for Edge {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        let this = cx.entity();
        let trigger = {
            let this = this.clone();
            on_activate_app(
                select_trigger("edge", "System", self.open, &colors),
                move |_, cx| {
                    this.update(cx, |h, cx| {
                        h.open = !h.open;
                        cx.notify();
                    })
                },
            )
        };
        let options = self.open.then(|| {
            (0..4)
                .map(|i| select_option(format!("edge-{i}"), format!("Option {i}"), i == 0, &colors))
                .collect()
        });
        let close = this.clone();
        let el = select(
            "edge",
            trigger,
            options,
            &colors,
            move |_, cx| {
                close.update(cx, |h, cx| {
                    h.open = false;
                    cx.notify();
                })
            },
            window,
            cx,
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .when(self.bottom, |d| d.justify_end())
            .child(el)
    }
}

fn open_edge(
    cx: &mut TestAppContext,
    bottom: bool,
) -> (gpui::Bounds<gpui::Pixels>, gpui::Bounds<gpui::Pixels>) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (_, cx) = cx.add_window_view(move |_, _| Edge {
        bottom,
        open: false,
    });
    click(cx, "edge");
    let trigger = cx.debug_bounds("edge").expect("trigger drawn");
    let list = cx.debug_bounds("edge-list").expect("list drawn");
    (trigger, list)
}

#[gpui::test]
fn select_list_hangs_under_the_trigger(cx: &mut TestAppContext) {
    let (trigger, list) = open_edge(cx, false);
    assert!(list.top() >= trigger.bottom(), "{trigger:?} {list:?}");
    assert_eq!(list.right(), trigger.right(), "right-aligned with it");
}

#[gpui::test]
fn select_list_flips_above_at_the_window_bottom(cx: &mut TestAppContext) {
    let (trigger, list) = open_edge(cx, true);
    assert!(list.bottom() <= trigger.top(), "{trigger:?} {list:?}");
}
