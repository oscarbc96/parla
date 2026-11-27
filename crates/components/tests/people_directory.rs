//! The People directory: every member from the member index, searchable by
//! name or title, with filters, cards, keyboard moves and the card actions.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use components::{
    PeopleDirectory, PeopleEvent, PeopleFilter, PeopleFilters, PeopleIndex, PeopleSource,
};
use domain::test_support::{member, uid};
use domain::{FixedNow, Member, UserId, WorkspaceState};
use gpui::{App, AppContext as _, Entity, TestAppContext, VisualTestContext};
use jiff::Timestamp;
use settings::WorkspaceKey;

/// 2026-11-15 10:00:00 UTC.
const NOW: i64 = 1_794_736_800;

struct Src {
    state: WorkspaceState,
    members_rev: u64,
    /// `(revision, id)` of single-member changes the source still knows.
    changes: Vec<(u64, UserId)>,
    watched: Vec<Vec<UserId>>,
}

impl PeopleSource for Src {
    fn people_state(&self) -> &WorkspaceState {
        &self.state
    }
    fn members_revision(&self) -> u64 {
        self.members_rev
    }
    fn members_changed_since(&self, since: u64) -> Option<Vec<UserId>> {
        let ids: Vec<UserId> = self
            .changes
            .iter()
            .filter(|(r, _)| *r > since)
            .map(|(_, u)| u.clone())
            .collect();
        // Every revision after `since` must be a known single change.
        (ids.len() as u64 == self.members_rev - since).then_some(ids)
    }
    fn clock(&self, _cx: &App) -> settings::Clock {
        settings::Clock::H24
    }
    fn watch_people(&mut self, people: Vec<UserId>, _cx: &mut App) {
        self.watched.push(people);
    }
}

fn person(id: &str, name: &str, title: Option<&str>, tz: Option<&str>) -> Member {
    let mut m = member(id, name);
    m.title = title.map(Arc::from);
    m.tz = tz.map(Arc::from);
    m
}

fn state(members: Vec<Member>) -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.name = Arc::from("Northwind Studio");
    s.self_user = Some(uid("me"));
    s.caps.presence_watch = true;
    s.caps.open_direct = true;
    for m in members {
        s.members.insert(m.id.clone(), m);
    }
    s
}

fn sample() -> Vec<Member> {
    let mut bot = member("b1", "Deploy Bot");
    bot.is_bot = true;
    let mut gone = member("x1", "Gone Person");
    gone.deleted = true;
    vec![
        person(
            "u1",
            "Tomás Silva",
            Some("Motion designer"),
            Some("Europe/Lisbon"),
        ),
        person(
            "u2",
            "aiko Tanaka",
            Some("Product designer"),
            Some("Asia/Tokyo"),
        ),
        person(
            "u3",
            "Jonas Weber",
            Some("Design engineer"),
            Some("Europe/Berlin"),
        ),
        person("u4", "Priya Raman", Some("Staff engineer"), None),
        person("me", "Me Myself", None, None),
        bot,
        gone,
    ]
}

fn names(idx: &PeopleIndex, rows: &[u32]) -> Vec<String> {
    rows.iter()
        .map(|&i| idx.entry(i as usize).name.to_string())
        .collect()
}

fn filter(idx: &PeopleIndex, q: &str, f: PeopleFilters) -> Vec<String> {
    let now = Timestamp::from_second(NOW).unwrap();
    let mine = jiff::tz::TimeZone::get("Europe/Lisbon").unwrap();
    names(idx, &idx.filter(q, &f, &mine, now))
}

#[test]
fn the_index_sorts_by_name_and_leaves_out_deleted_people_and_me() {
    let st = state(sample());
    let idx = PeopleIndex::build(&st);
    assert_eq!(
        filter(&idx, "", PeopleFilters::default()),
        ["aiko Tanaka", "Jonas Weber", "Priya Raman", "Tomás Silva"]
    );
    assert!(idx.has_bots());
    assert!(idx.has_zones());
}

#[test]
fn search_matches_every_word_in_the_name_or_title_ignoring_case() {
    let st = state(sample());
    let idx = PeopleIndex::build(&st);
    let f = PeopleFilters::default();
    assert_eq!(
        filter(&idx, "DESIGN", f),
        ["aiko Tanaka", "Jonas Weber", "Tomás Silva"]
    );
    assert_eq!(filter(&idx, "design eng", f), ["Jonas Weber"]);
    assert_eq!(filter(&idx, "tomas", f), Vec::<String>::new());
    assert_eq!(filter(&idx, "tomás", f), ["Tomás Silva"]);
    assert_eq!(filter(&idx, "  priya  ", f), ["Priya Raman"]);
}

#[test]
fn bots_are_hidden_until_the_filter_shows_them() {
    let st = state(sample());
    let idx = PeopleIndex::build(&st);
    let f = PeopleFilters {
        bots: true,
        ..Default::default()
    };
    assert_eq!(
        filter(&idx, "deploy", PeopleFilters::default()),
        Vec::<String>::new()
    );
    assert_eq!(filter(&idx, "deploy", f), ["Deploy Bot"]);
}

#[test]
fn my_time_zone_keeps_people_on_my_clock() {
    let st = state(sample());
    let idx = PeopleIndex::build(&st);
    let f = PeopleFilters {
        my_time_zone: true,
        ..Default::default()
    };
    // Lisbon is on UTC in November; nobody else in the sample is.
    assert_eq!(filter(&idx, "", f), ["Tomás Silva"]);
}

#[test]
fn one_member_change_patches_the_index_in_place() {
    let mut st = state(sample());
    let mut idx = PeopleIndex::build(&st);
    // Renamed (moves in the order), retitled, turned into a bot, removed.
    st.members.insert(
        uid("u4"),
        person("u4", "Ana Priya", Some("Design lead"), None),
    );
    st.members.get_mut(&uid("u3")).unwrap().deleted = true;
    let mut b = person("u9", "Zed New", None, Some("Asia/Kolkata"));
    b.is_bot = false;
    st.members.insert(uid("u9"), b);
    idx.update(&st, &[uid("u4"), uid("u3"), uid("u9")]);
    let f = PeopleFilters::default();
    assert_eq!(
        filter(&idx, "", f),
        ["aiko Tanaka", "Ana Priya", "Tomás Silva", "Zed New"]
    );
    assert_eq!(filter(&idx, "design lead", f), ["Ana Priya"]);
    assert_eq!(idx.humans(), 4);
    let rebuilt = PeopleIndex::build(&st);
    assert_eq!(
        filter(&idx, "", PeopleFilters { bots: true, ..f }),
        filter(&rebuilt, "", PeopleFilters { bots: true, ..f })
    );
}

fn big_state(n: usize) -> WorkspaceState {
    let zones = [
        "Europe/Madrid",
        "Asia/Tokyo",
        "America/New_York",
        "Asia/Kolkata",
    ];
    let titles = [
        "Product designer",
        "Staff engineer",
        "Support lead",
        "Data analyst",
    ];
    state(
        (0..n)
            .map(|i| {
                person(
                    &format!("U{i:05}"),
                    &format!("Person {i:05} Surname{}", i % 97),
                    Some(titles[i % titles.len()]),
                    Some(zones[i % zones.len()]),
                )
            })
            .collect(),
    )
}

/// Perf-style: a keystroke in a 10k directory refilters well inside a frame.
#[test]
#[ignore = "perf: run by `mise run test:perf`"]
fn filtering_ten_thousand_members_is_fast() {
    let st = big_state(10_000);
    let now = Timestamp::from_second(NOW).unwrap();
    let mine = jiff::tz::TimeZone::get("Europe/Madrid").unwrap();
    let queries = [
        "p",
        "pe",
        "per",
        "pers",
        "person 0",
        "surname4",
        "staff",
        "design lead",
        "zz",
    ];
    let f = PeopleFilters {
        my_time_zone: true,
        ..Default::default()
    };
    // Best of several rounds: a busy machine only ever makes a round slower.
    let (mut build, mut each, mut hits) = (Duration::MAX, Duration::MAX, 0);
    for _ in 0..5 {
        let t = Instant::now();
        let idx = PeopleIndex::build(&st);
        build = build.min(t.elapsed());
        let t = Instant::now();
        hits = 0;
        for q in queries {
            hits += idx.filter(q, &PeopleFilters::default(), &mine, now).len();
            hits += idx.filter(q, &f, &mine, now).len();
        }
        each = each.min(t.elapsed() / (queries.len() as u32 * 2));
    }
    eprintln!("people index build {build:?}, filter {each:?} per query ({hits} hits)");
    assert!(build.as_millis() < 200, "index build took {build:?}");
    assert!(each.as_micros() < 8_000, "a filter took {each:?}");
}

/// Perf-style: one member's change in a 10k directory costs less than a
/// frame (patch plus the refilter it causes), best of 5.
#[test]
#[ignore = "perf: run by `mise run test:perf`"]
fn one_member_change_in_ten_thousand_is_under_a_frame() {
    let mut st = big_state(10_000);
    let mut idx = PeopleIndex::build(&st);
    let now = Timestamp::from_second(NOW).unwrap();
    let mine = jiff::tz::TimeZone::get("Europe/Madrid").unwrap();
    let mut best = Duration::MAX;
    for round in 0..5 {
        let id = uid(&format!("U{:05}", 4_000 + round));
        st.members.insert(
            id.clone(),
            person(
                &id.0,
                &format!("Aaron Renamed {round}"),
                Some("Design lead"),
                Some("Europe/Lisbon"),
            ),
        );
        let t = Instant::now();
        idx.update(&st, std::slice::from_ref(&id));
        let rows = idx.filter("", &PeopleFilters::default(), &mine, now);
        best = best.min(t.elapsed());
        assert_eq!(rows.len(), 10_000);
    }
    eprintln!("people index one-member update {best:?}");
    assert!(best.as_micros() < 8_000, "an update took {best:?}");
}

fn setup(
    cx: &mut TestAppContext,
    st: WorkspaceState,
) -> (
    Entity<Src>,
    Entity<PeopleDirectory<Src>>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        i18n::gpui::init(cx, &[components::catalog()], i18n::LocalePref::System);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
        components::set_now(cx, Arc::new(FixedNow(Timestamp::from_second(NOW).unwrap())));
        components::set_time_zone(cx, jiff::tz::TimeZone::get("Europe/Lisbon").unwrap());
    });
    let src = cx.new(|_| Src {
        state: st,
        members_rev: 1,
        changes: vec![],
        watched: vec![],
    });
    let s = src.clone();
    let (view, vcx) = cx.add_window_view(move |_w, cx| PeopleDirectory::new(s, cx));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    (src, view, vcx)
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    vcx.run_until_parked();
}

fn events(
    view: &Entity<PeopleDirectory<Src>>,
    vcx: &mut VisualTestContext,
) -> std::rc::Rc<std::cell::RefCell<Vec<PeopleEvent>>> {
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let l = log.clone();
    vcx.update(|_, cx| {
        cx.subscribe(view, move |_, ev: &PeopleEvent, _| {
            l.borrow_mut().push(ev.clone())
        })
        .detach();
    });
    log
}

#[gpui::test]
fn cards_show_name_title_and_local_time(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, state(sample()));
    assert!(vcx.debug_bounds("people-card-0").is_some());
    assert!(vcx.debug_bounds("people-card-3").is_some());
    assert!(vcx.debug_bounds("people-card-4").is_none());
    let card = vcx.update(|_, cx| view.update(cx, |v, cx| v.card_text(0, cx)));
    assert_eq!(
        card,
        Some((
            "aiko Tanaka".into(),
            "Product designer".into(),
            "19:00 · Tokyo".into()
        ))
    );
    assert_eq!(
        vcx.update(|_, cx| view.update(cx, |v, cx| v.header_count(cx))),
        "4 in Northwind Studio"
    );
}

#[gpui::test]
fn the_count_without_a_workspace_name_is_plural_aware(cx: &mut TestAppContext) {
    let mut st = state(vec![person("u1", "Ana", None, None)]);
    st.name = Arc::from("");
    let (src, view, vcx) = setup(cx, st);
    assert_eq!(
        vcx.update(|_, cx| view.update(cx, |v, cx| v.header_count(cx))),
        "1 person"
    );
    src.update(vcx, |s, cx| {
        s.state
            .members
            .insert(uid("u2"), person("u2", "Bo", None, None));
        s.members_rev += 1;
        cx.notify();
    });
    assert_eq!(
        vcx.update(|_, cx| view.update(cx, |v, cx| v.header_count(cx))),
        "2 people"
    );
}

#[gpui::test]
fn redraws_format_no_time_and_reuse_the_cache(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, state(sample()));
    let counts = |vcx: &mut VisualTestContext| {
        view.read_with(vcx, |v, _| {
            (v.index_builds(), v.filter_runs(), v.time_formats())
        })
    };
    let first = counts(vcx);
    assert!(first.2 > 0, "the first frame formats the times");
    for _ in 0..3 {
        draw(vcx);
    }
    assert_eq!(counts(vcx), first, "redraws reuse the cache");
}

#[gpui::test]
fn the_filtered_rows_are_cached_until_the_query_filters_or_members_change(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, state(sample()));
    let runs = |vcx: &mut VisualTestContext| {
        view.read_with(vcx, |v, _| (v.index_builds(), v.filter_runs()))
    };
    let first = runs(vcx);
    vcx.update(|_, cx| view.update(cx, |v, cx| v.set_query("design", cx)));
    draw(vcx);
    assert_eq!(runs(vcx), (first.0, first.1 + 1));
    // A full members change (no single-member log) rebuilds.
    src.update(vcx, |s, cx| {
        s.state
            .members
            .insert(uid("u9"), person("u9", "Zed Designer", None, None));
        s.members_rev += 1;
        cx.notify();
    });
    draw(vcx);
    assert_eq!(runs(vcx), (first.0 + 1, first.1 + 2));
    assert_eq!(
        view.update(vcx, |v, cx| v.results(cx)),
        [uid("u2"), uid("u3"), uid("u1"), uid("u9")]
    );
}

#[gpui::test]
fn one_member_update_is_patched_not_rebuilt(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, state(sample()));
    let builds = view.read_with(vcx, |v, _| v.index_builds());
    src.update(vcx, |s, cx| {
        s.state.members.insert(
            uid("u4"),
            person("u4", "Priya Raman", Some("Design lead"), None),
        );
        s.members_rev += 1;
        let rev = s.members_rev;
        s.changes.push((rev, uid("u4")));
        cx.notify();
    });
    view.update(vcx, |v, cx| v.set_query("design lead", cx));
    assert_eq!(view.update(vcx, |v, cx| v.results(cx)), [uid("u4")]);
    view.read_with(vcx, |v, _| {
        assert_eq!(v.index_builds(), builds);
        assert_eq!(v.index_patches(), 1);
    });
}

#[gpui::test]
fn a_search_with_no_match_shows_the_empty_state_and_unwatches(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, state(sample()));
    vcx.update(|_, cx| view.update(cx, |v, cx| v.set_query("nobody here", cx)));
    draw(vcx);
    assert!(vcx.debug_bounds("people-empty").is_some());
    assert!(vcx.debug_bounds("people-card-0").is_none());
    assert_eq!(
        src.read_with(vcx, |s, _| s.watched.last().cloned()),
        Some(vec![])
    );
}

#[gpui::test]
fn down_on_no_results_keeps_the_search_field(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, state(sample()));
    view.update(vcx, |v, cx| v.set_query("nobody here", cx));
    draw(vcx);
    let search = vcx.update(|_, cx| view.read(cx).search_focus(cx));
    vcx.update(|w, cx| w.focus(&search, cx));
    vcx.simulate_keystrokes("down");
    assert!(vcx.update(|w, _| search.is_focused(w)));
}

#[gpui::test]
fn filter_chips_toggle_and_show_only_what_the_data_supports(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, state(sample()));
    // Presence is known only for watched people: no "Active now".
    assert!(vcx.debug_bounds("people-filter-active").is_none());
    assert!(vcx.debug_bounds("people-filter-zone").is_some());
    assert!(vcx.debug_bounds("people-filter-bots").is_some());
    let b = vcx.debug_bounds("people-filter-bots").unwrap();
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert!(view.read_with(vcx, |v, _| v.filters().bots));
    assert!(view.update(vcx, |v, cx| v.results(cx)).contains(&uid("b1")));
    // Without zones or bots the chips are not offered.
    src.update(vcx, |s, cx| {
        s.state.members.retain(|u, _| u.0.as_ref() == "u4");
        s.members_rev += 1;
        cx.notify();
    });
    draw(vcx);
    assert!(vcx.debug_bounds("people-filter-zone").is_none());
    assert!(vcx.debug_bounds("people-filter-bots").is_none());
    view.update(vcx, |v, cx| v.toggle_filter(PeopleFilter::Bots, cx));
    assert!(!view.read_with(vcx, |v, _| v.filters().bots));
}

#[gpui::test]
fn the_partial_note_shows_only_when_the_provider_says_so(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup(cx, state(sample()));
    assert!(vcx.debug_bounds("people-partial").is_none());
    src.update(vcx, |s, cx| {
        s.state.caps.members_partial = true;
        cx.notify();
    });
    draw(vcx);
    assert!(vcx.debug_bounds("people-partial").is_some());
}

#[gpui::test]
fn arrow_keys_move_between_cards_and_enter_messages(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, state(sample()));
    let log = events(&view, vcx);
    let grid = view.read_with(vcx, |v, _| v.grid_focus());
    vcx.update(|w, cx| w.focus(&grid, cx));
    vcx.simulate_keystrokes("right right down");
    // Three cards a row: 0 -> 1 -> 2, then down past the end clamps to 3.
    assert_eq!(view.read_with(vcx, |v, _| v.cursor()), 3);
    vcx.simulate_keystrokes("up right");
    assert_eq!(view.read_with(vcx, |v, _| v.cursor()), 1);
    vcx.simulate_keystrokes("enter");
    vcx.simulate_keystrokes("space");
    let jonas = PeopleEvent::Message {
        user: uid("u3"),
        name: "Jonas Weber".into(),
    };
    assert_eq!(*log.borrow(), [jonas.clone(), jonas]);
}

#[gpui::test]
fn enter_brings_the_cursor_card_into_view(cx: &mut TestAppContext) {
    let many: Vec<Member> = (0..90)
        .map(|i| person(&format!("u{i:02}"), &format!("Person {i:02}"), None, None))
        .collect();
    let (_src, view, vcx) = setup(cx, state(many));
    let log = events(&view, vcx);
    let grid = view.read_with(vcx, |v, _| v.grid_focus());
    vcx.update(|w, cx| w.focus(&grid, cx));
    vcx.simulate_keystrokes("end");
    draw(vcx);
    assert!(vcx.debug_bounds("people-card-89").is_some(), "end scrolls");
    view.update(vcx, |v, _| v.scroll_to_row(0));
    draw(vcx);
    assert!(vcx.debug_bounds("people-card-89").is_none());
    vcx.simulate_keystrokes("enter");
    draw(vcx);
    assert!(
        vcx.debug_bounds("people-card-89").is_some(),
        "enter scrolls"
    );
    assert_eq!(
        *log.borrow(),
        [PeopleEvent::Message {
            user: uid("u89"),
            name: "Person 89".into()
        }]
    );
}

#[gpui::test]
fn card_buttons_message_and_view_profile(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, state(sample()));
    let log = events(&view, vcx);
    let b = vcx.debug_bounds("people-message-2").expect("message drawn");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    let b = vcx.debug_bounds("people-profile-0").expect("profile drawn");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(
        *log.borrow(),
        [
            PeopleEvent::Message {
                user: uid("u4"),
                name: "Priya Raman".into()
            },
            PeopleEvent::ViewProfile(uid("u2"))
        ]
    );
}

#[gpui::test]
fn escape_closes_and_the_cards_on_screen_are_watched(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, state(sample()));
    let log = events(&view, vcx);
    vcx.run_until_parked();
    let watched = src.read_with(vcx, |s, _| s.watched.last().cloned());
    assert_eq!(
        watched,
        Some(vec![uid("u2"), uid("u3"), uid("u4"), uid("u1")])
    );
    let grid = view.read_with(vcx, |v, _| v.grid_focus());
    vcx.update(|w, cx| w.focus(&grid, cx));
    vcx.simulate_keystrokes("escape");
    assert_eq!(*log.borrow(), [PeopleEvent::Close]);
}
