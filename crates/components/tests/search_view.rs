mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use components::{
    SearchPanel, SearchPanelEvent, SearchResults, SearchRow, SearchScope, SearchSource,
    SearchTarget,
};
use domain::test_support::{cid, mid, uid};
use gpui::{App, AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};

struct Src {
    calls: Vec<(SearchScope, String)>,
    results: SearchResults,
    remote: bool,
}

impl SearchSource for Src {
    fn search(&mut self, scope: SearchScope, input: String, _: &mut App) {
        self.calls.push((scope, input));
    }
    fn search_results(&self) -> &SearchResults {
        &self.results
    }
    fn remote_search(&self) -> bool {
        self.remote
    }
    fn provider_name(&self) -> &str {
        "Fakechat"
    }
}

fn row(ts: &str, text: &str) -> SearchRow {
    SearchRow::message(mid("C1", ts), "#general", "Bob", "10:13", text)
}

type Events = Rc<RefCell<Vec<SearchPanelEvent>>>;

fn setup<'a>(
    cx: &'a mut TestAppContext,
    remote: bool,
    prefill: Option<&str>,
) -> (
    Entity<Src>,
    Entity<SearchPanel<Src>>,
    &'a mut VisualTestContext,
    Events,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let src = cx.new(|_| Src {
        calls: vec![],
        results: SearchResults::default(),
        remote,
    });
    let s = src.clone();
    let name = prefill.map(str::to_string);
    let (view, vcx) = cx.add_window_view(move |_, cx| match &name {
        Some(n) => SearchPanel::in_conversation(s, n, cx),
        None => SearchPanel::new(s, cx),
    });
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

#[gpui::test]
fn local_hits_first_then_remote(cx: &mut TestAppContext) {
    let (src, view, vcx, events) = setup(cx, true, None);
    src.update(vcx, |s, _| {
        s.results = SearchResults {
            query: "deploy".into(),
            rows: vec![row("1.000001", "local")],
            remote: vec![row("2.000001", "remote one"), row("3.000001", "remote two")],
            ..Default::default()
        }
    });
    view.update(vcx, |_, cx| cx.notify());
    let order: Vec<String> = view.read_with(vcx, |v, cx| {
        v.hits(cx).iter().map(|r| r.snippet.to_string()).collect()
    });
    assert_eq!(order, ["local", "remote one", "remote two"]);
    vcx.simulate_keystrokes("down down enter");
    assert_eq!(
        *events.borrow(),
        [SearchPanelEvent::Open(mid("C1", "3.000001"))]
    );
}

#[gpui::test]
fn offline_shows_local_only_with_note(cx: &mut TestAppContext) {
    let (src, view, vcx, _) = setup(cx, true, None);
    assert!(view.read_with(vcx, |v, cx| v.notes(cx).is_empty()));
    src.update(vcx, |s, _| {
        s.results = SearchResults {
            query: "x".into(),
            rows: vec![row("1.000001", "local")],
            offline: true,
            ..Default::default()
        }
    });
    let notes = view.read_with(vcx, |v, cx| v.notes(cx));
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("Offline"), "{notes:?}");
}

#[gpui::test]
fn remote_failure_keeps_local_rows_and_says_so(cx: &mut TestAppContext) {
    let (src, view, vcx, _) = setup(cx, true, None);
    src.update(vcx, |s, _| {
        s.results = SearchResults {
            query: "x".into(),
            rows: vec![row("1.000001", "local")],
            remote_error: true,
            ..Default::default()
        }
    });
    let notes = view.read_with(vcx, |v, cx| v.notes(cx));
    assert!(notes[0].contains("server search failed"), "{notes:?}");
    assert_eq!(view.read_with(vcx, |v, cx| v.hits(cx).len()), 1);
}

#[gpui::test]
fn prefills_current_conversation(cx: &mut TestAppContext) {
    let (_src, view, vcx, _) = setup(cx, false, Some("#general"));
    assert_eq!(
        view.read_with(vcx, |v, _| v.query().to_string()),
        "in:#general "
    );
}

#[gpui::test]
fn warnings_shown_for_unknown_names(cx: &mut TestAppContext) {
    let (src, view, vcx, _) = setup(cx, false, None);
    src.update(vcx, |s, _| {
        s.results = SearchResults {
            query: "x".into(),
            warnings: vec!["in:#nope".into(), "from:@ghost".into()],
            ..Default::default()
        }
    });
    let notes = view.read_with(vcx, |v, cx| v.notes(cx));
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("in:#nope, from:@ghost"), "{notes:?}");
}

#[gpui::test]
fn scope_tabs_need_the_search_capability(cx: &mut TestAppContext) {
    let (_s, local, vcx, _) = setup(cx, false, None);
    assert_eq!(
        local.read_with(vcx, |v, cx| v.scopes(cx)),
        [SearchScope::Messages]
    );
    // Without server search the other tabs cannot be selected either.
    local.update(vcx, |v, cx| v.set_scope(SearchScope::People, cx));
    assert_eq!(
        local.read_with(vcx, |v, _| v.scope()),
        SearchScope::Messages
    );
}

#[gpui::test]
fn switching_scope_searches_again(cx: &mut TestAppContext) {
    let (src, view, vcx, _) = setup(cx, true, None);
    view.update(vcx, |v, cx| v.set_query("ana", cx));
    view.update(vcx, |v, cx| v.set_scope(SearchScope::People, cx));
    vcx.executor().advance_clock(Duration::from_millis(400));
    vcx.run_until_parked();
    assert_eq!(
        src.read_with(vcx, |s, _| s.calls.clone()),
        [(SearchScope::People, "ana".to_string())]
    );
}

#[gpui::test]
fn each_kind_of_hit_opens_its_target(cx: &mut TestAppContext) {
    let (src, view, vcx, events) = setup(cx, true, None);
    let mut person = row("1.000001", "Ana");
    person.target = SearchTarget::Person(uid("u2"));
    let mut conv = row("1.000001", "general");
    conv.target = SearchTarget::Conversation(cid("C1"));
    src.update(vcx, |s, _| {
        s.results = SearchResults {
            query: "a".into(),
            rows: vec![person, conv],
            ..Default::default()
        }
    });
    view.update(vcx, |_, cx| cx.notify());
    vcx.simulate_keystrokes("enter down enter");
    assert_eq!(
        *events.borrow(),
        [
            SearchPanelEvent::OpenPerson(uid("u2")),
            SearchPanelEvent::OpenConversation(cid("C1"))
        ]
    );
}
