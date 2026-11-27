mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use components::{
    PreviewLine, SearchPanel, SearchPanelEvent, SearchResults, SearchRow, SearchScope,
    SearchSource, format_date_time, highlight_spans, resolve_user_ids, results_label,
};
use domain::test_support::mid;
use gpui::{App, AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};

struct Src {
    queries: Vec<String>,
    results: SearchResults,
    /// Messages around `1.000001`, the only hit whose context is known.
    context: Vec<PreviewLine>,
    loads: Vec<Option<domain::MessageId>>,
}

impl SearchSource for Src {
    fn search(&mut self, _: SearchScope, query: String, _: &mut App) {
        self.results.query = query.clone().into();
        self.queries.push(query);
    }
    fn search_results(&self) -> &SearchResults {
        &self.results
    }
    fn context(&self, id: &domain::MessageId, _: &App) -> Vec<PreviewLine> {
        if self.context.iter().any(|l| l.id == *id) {
            self.context.clone()
        } else {
            Vec::new()
        }
    }
    fn load_context(&mut self, id: Option<domain::MessageId>, _: &mut App) {
        self.loads.push(id);
    }
}

fn line(ts: &str, text: &str) -> PreviewLine {
    PreviewLine {
        id: mid("C1", ts),
        author: "Ana".into(),
        avatar: None,
        time: "10:00".into(),
        text: text.to_string().into(),
    }
}

fn row(ts: &str, text: &str) -> SearchRow {
    SearchRow::message(mid("C1", ts), "#general", "Bob", "10:13", text)
}

type Events = Rc<RefCell<Vec<SearchPanelEvent>>>;
type Setup<'a> = (
    Entity<Src>,
    Entity<SearchPanel<Src>>,
    &'a mut VisualTestContext,
    Events,
);

fn setup(cx: &mut TestAppContext) -> Setup<'_> {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let src = cx.new(|_| Src {
        queries: vec![],
        results: SearchResults::default(),
        context: vec![
            line("0.500001", "before"),
            line("1.000001", "deploy one"),
            line("1.500001", "after"),
        ],
        loads: vec![],
    });
    let s = src.clone();
    let (view, vcx) = cx.add_window_view(|_, cx| SearchPanel::new(s, cx));
    let events = Rc::new(RefCell::new(Vec::new()));
    let e = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &SearchPanelEvent, _| {
            e.borrow_mut().push(ev.clone())
        })
        .detach();
    });
    vcx.update(|w, cx| {
        let h = view.read(cx).focus_handle(cx);
        w.focus(&h, cx)
    });
    (src, view, vcx, events)
}

#[test]
fn highlights_whole_words_and_prefixes() {
    let s = "The deploy finished, deployed twice";
    assert_eq!(highlight_spans(s, "deploy"), vec![4..10]);
    assert_eq!(highlight_spans(s, "DEPLOY*"), vec![4..10, 21..29]);
    assert_eq!(
        highlight_spans(s, "\"finished\" twice"),
        vec![11..19, 30..35]
    );
    assert!(highlight_spans(s, "").is_empty());
}

#[test]
fn highlights_use_byte_offsets_with_unicode() {
    let s = "r\u{e9}sum\u{e9} deploy";
    assert_eq!(highlight_spans(s, "deploy"), vec![9..15]);
}

#[test]
fn result_count_label_is_plural_aware() {
    assert_eq!(results_label(1, &common::en()), "1 result");
    assert_eq!(results_label(3, &common::en()), "3 results");
}

#[gpui::test]
fn typing_is_debounced_into_one_search(cx: &mut TestAppContext) {
    let (src, _view, vcx, _) = setup(cx);
    vcx.simulate_input("dep");
    vcx.executor().advance_clock(Duration::from_millis(100));
    vcx.simulate_input("loy");
    vcx.executor().advance_clock(Duration::from_millis(100));
    assert!(src.read_with(vcx, |s, _| s.queries.is_empty()));
    vcx.executor().advance_clock(Duration::from_millis(300));
    vcx.run_until_parked();
    assert_eq!(src.read_with(vcx, |s, _| s.queries.clone()), ["deploy"]);
}

#[gpui::test]
fn clearing_the_query_clears_at_once(cx: &mut TestAppContext) {
    let (src, _view, vcx, _) = setup(cx);
    vcx.simulate_input("a");
    vcx.simulate_keystrokes("backspace");
    vcx.run_until_parked();
    assert_eq!(src.read_with(vcx, |s, _| s.queries.clone()), [""]);
}

#[gpui::test]
fn enter_opens_the_selected_hit_and_arrows_move(cx: &mut TestAppContext) {
    let (src, view, vcx, events) = setup(cx);
    src.update(vcx, |s, _| {
        s.results = SearchResults {
            query: "deploy".into(),
            rows: vec![row("1.000001", "deploy one"), row("2.000001", "deploy two")],
            ..Default::default()
        }
    });
    view.update(vcx, |_, cx| cx.notify());
    vcx.simulate_keystrokes("down enter");
    assert_eq!(
        *events.borrow(),
        [SearchPanelEvent::Open(mid("C1", "2.000001"))]
    );
    vcx.simulate_keystrokes("up enter");
    assert_eq!(
        events.borrow()[1],
        SearchPanelEvent::Open(mid("C1", "1.000001"))
    );
}

#[gpui::test]
fn escape_closes(cx: &mut TestAppContext) {
    let (_src, _view, vcx, events) = setup(cx);
    vcx.simulate_keystrokes("escape");
    assert_eq!(*events.borrow(), [SearchPanelEvent::Close]);
}

#[gpui::test]
fn enter_without_hits_does_nothing(cx: &mut TestAppContext) {
    let (_src, _view, vcx, events) = setup(cx);
    vcx.simulate_keystrokes("enter");
    assert!(events.borrow().is_empty());
}

#[test]
fn date_time_for_hits_spanning_days() {
    let t = domain::test_support::ts("1700000000.000100");
    let utc = jiff::tz::TimeZone::UTC;
    assert_eq!(
        format_date_time(&t, settings::Clock::H24, &utc, &common::en()),
        "Nov 14, 2023, 22:13"
    );
}

#[test]
fn user_ids_in_snippets_become_names() {
    let name = |id: &str| match id {
        "U0000001" => Some("Ana Reyes".to_string()),
        "U0000002" => Some("Bob".to_string()),
        _ => None,
    };
    assert_eq!(
        resolve_user_ids("rolled back @U0000001", name),
        "rolled back @Ana Reyes"
    );
    assert_eq!(
        resolve_user_ids("@U0000001, @U0000002: done. cc @U0000009!", name),
        "@Ana Reyes, @Bob: done. cc @U0000009!"
    );
    // Only whole tokens after an at sign: no e-mail addresses, no partial ids.
    assert_eq!(
        resolve_user_ids("mail bob@U0000001 or @U00000011", name),
        "mail bob@U0000001 or @U00000011"
    );
    assert_eq!(resolve_user_ids("no mentions", name), "no mentions");
}

#[gpui::test]
fn the_field_has_a_caret_and_deletes_whole_graphemes(cx: &mut TestAppContext) {
    let (_src, view, vcx, _) = setup(cx);
    vcx.simulate_input("abc");
    vcx.simulate_keystrokes("left backspace");
    assert_eq!(view.read_with(vcx, |v, _| v.query().to_string()), "ac");
    vcx.simulate_input("e\u{301}");
    vcx.simulate_keystrokes("backspace");
    assert_eq!(view.read_with(vcx, |v, _| v.query().to_string()), "ac");
}

#[gpui::test]
fn pasting_flattens_lines_and_searches(cx: &mut TestAppContext) {
    let (src, view, vcx, _) = setup(cx);
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string("one\ntwo \n".into()));
    vcx.dispatch_action(components::text_input::Paste);
    vcx.executor().advance_clock(Duration::from_millis(300));
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |v, _| v.query().to_string()), "one two");
    assert_eq!(src.read_with(vcx, |s, _| s.queries.clone()), ["one two"]);
}

#[gpui::test]
fn the_selected_hit_shows_in_context_and_jumps(cx: &mut TestAppContext) {
    let (src, view, vcx, events) = setup(cx);
    view.update(vcx, |v, cx| v.set_query("deploy", cx));
    src.update(vcx, |s, cx| {
        s.results = SearchResults {
            query: "deploy".into(),
            rows: vec![row("1.000001", "deploy one"), row("2.000001", "deploy two")],
            ..Default::default()
        };
        cx.notify();
    });
    vcx.run_until_parked();
    let texts = |view: &Entity<SearchPanel<Src>>, vcx: &mut VisualTestContext| {
        view.read_with(vcx, |v, _| {
            v.preview()
                .iter()
                .map(|l| l.text.to_string())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(texts(&view, vcx), ["before", "deploy one", "after"]);
    assert!(vcx.debug_bounds("search-preview").is_some());
    assert!(vcx.debug_bounds("search-jump").is_some());
    // Down moves to a hit whose context is not held: it is asked for (the
    // session drops the read of the hit before), and the hit shows meanwhile.
    vcx.simulate_keystrokes("down");
    assert_eq!(
        src.read_with(vcx, |s, _| s.loads.last().cloned()),
        Some(Some(mid("C1", "2.000001")))
    );
    assert_eq!(texts(&view, vcx), ["deploy two"]);
    assert!(events.borrow().is_empty(), "moving does not open");
    let jump = vcx.debug_bounds("search-jump").expect("jump button");
    vcx.simulate_click(jump.center(), gpui::Modifiers::none());
    assert_eq!(
        *events.borrow(),
        [SearchPanelEvent::Open(mid("C1", "2.000001"))]
    );
}

#[gpui::test]
fn a_conversation_hit_keeps_the_context_pane_as_a_placeholder(cx: &mut TestAppContext) {
    let (src, view, vcx, _events) = setup(cx);
    view.update(vcx, |v, cx| v.set_query("gen", cx));
    src.update(vcx, |s, cx| {
        s.results = SearchResults {
            query: "gen".into(),
            rows: vec![
                row("1.000001", "deploy one"),
                SearchRow {
                    target: components::SearchTarget::Conversation(domain::test_support::cid("C2")),
                    conversation: "#general".into(),
                    author: "".into(),
                    time: "".into(),
                    snippet: "#general".into(),
                },
            ],
            ..Default::default()
        };
        cx.notify();
    });
    vcx.run_until_parked();
    let pane = vcx.debug_bounds("search-preview").expect("pane").size.width;
    vcx.simulate_keystrokes("down");
    vcx.run_until_parked();
    let placeholder = vcx
        .debug_bounds("search-preview")
        .expect("the pane stays for a conversation hit");
    assert_eq!(placeholder.size.width, pane, "the width does not jump");
    assert!(vcx.debug_bounds("search-preview-empty").is_some());
    assert!(vcx.debug_bounds("search-jump").is_none());
    assert_eq!(
        src.read_with(vcx, |s, _| s.loads.last().cloned()),
        Some(None),
        "the read of the message hit is dropped"
    );
}

#[test]
fn the_context_pane_yields_before_the_timeline() {
    use gpui::px;
    use themes::metrics::{LAYOUT_DETAIL_PANEL, LAYOUT_RAIL, LAYOUT_SIDEBAR};
    let right_of_sidebar = |w: f32| px(w) - LAYOUT_RAIL - LAYOUT_SIDEBAR;
    // The timeline keeps what it had beside the hits alone.
    for w in [800.0, 1134.0] {
        assert_eq!(
            components::search_context_width(right_of_sidebar(w)),
            None,
            "{w}"
        );
    }
    let wide = components::search_context_width(right_of_sidebar(1600.0));
    assert_eq!(wide, Some(LAYOUT_DETAIL_PANEL));
    let mid = components::search_context_width(right_of_sidebar(1440.0)).expect("shown");
    assert!(mid < LAYOUT_DETAIL_PANEL);
    assert!(
        right_of_sidebar(1440.0) - LAYOUT_DETAIL_PANEL - mid
            >= themes::metrics::LAYOUT_TIMELINE_MIN
    );
}
