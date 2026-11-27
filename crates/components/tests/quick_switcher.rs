use std::cell::RefCell;
use std::rc::Rc;

use components::{
    PickerDelegate as _, PickerEvent, QuickSwitcher, SwitchDelegate, SwitchItem, SwitchTarget, rank,
};
use domain::test_support::cid;
use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use settings::WorkspaceKey;

fn ws(id: &str) -> WorkspaceKey {
    WorkspaceKey::new("fake", id)
}

fn item(w: &str, c: &str, label: &str, unread: bool) -> SwitchItem {
    SwitchItem {
        workspace: ws(w),
        target: SwitchTarget::Conversation(cid(c)),
        label: label.to_string().into(),
        workspace_name: w.to_string().into(),
        unread,
        mentions: 0,
        joined: true,
    }
}

fn labels(items: &[SwitchItem], order: &[usize]) -> Vec<String> {
    order.iter().map(|i| items[*i].label.to_string()).collect()
}

#[test]
fn rank_prefers_recent_then_unread() {
    // Same fuzzy score for "ge": the recent one leads, then the unread one.
    let items = vec![
        item("w1", "C1", "gen-a", false),
        item("w1", "C2", "gen-b", true),
        item("w1", "C3", "gen-c", false),
    ];
    let recent = vec![(ws("w1"), cid("C3"))];
    let order = rank(&items, "gen", &recent);
    assert_eq!(labels(&items, &order), ["gen-c", "gen-b", "gen-a"]);
    // A better fuzzy match still beats recency.
    let items = vec![
        item("w1", "C1", "general", false),
        item("w1", "C2", "big-news-gen", false),
    ];
    let order = rank(&items, "gen", &[(ws("w1"), cid("C2"))]);
    assert_eq!(labels(&items, &order), ["general", "big-news-gen"]);
}

#[test]
fn empty_query_lists_recent() {
    let items = vec![
        item("w1", "C1", "alpha", false),
        item("w1", "C2", "beta", true),
        item("w2", "C9", "gamma", false),
        item("w1", "C4", "delta", false),
    ];
    // Newest first: gamma (other workspace), then delta; then unread; then the rest in order.
    let recent = vec![(ws("w2"), cid("C9")), (ws("w1"), cid("C4"))];
    let order = rank(&items, "", &recent);
    assert_eq!(labels(&items, &order), ["gamma", "delta", "beta", "alpha"]);
}

#[test]
fn recent_is_per_workspace() {
    // The same conversation id in two workspaces: only the recent workspace's one is recent.
    let items = vec![item("w1", "D1", "dm", false), item("w2", "D1", "dm", false)];
    let order = rank(&items, "dm", &[(ws("w2"), cid("D1"))]);
    assert_eq!(order, [1, 0]);
}

#[test]
fn active_workspace_wins_ties() {
    // Callers list the active workspace first; equal scores keep that order.
    let items = vec![
        item("w2", "C1", "ops", false),
        item("w1", "C1", "ops", false),
    ];
    assert_eq!(rank(&items, "ops", &[]), [0, 1]);
}

#[test]
fn non_matches_are_left_out() {
    let items = vec![
        item("w1", "C1", "general", false),
        item("w1", "C2", "random", false),
    ];
    assert_eq!(rank(&items, "zzz", &[]), Vec::<usize>::new());
}

type Events = Rc<RefCell<Vec<PickerEvent>>>;

fn setup(cx: &mut TestAppContext) -> (Entity<QuickSwitcher>, &mut VisualTestContext, Events) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let items = vec![
        item("w1", "C1", "general", false),
        item("w1", "C2", "random", true),
        item("w2", "C1", "general", false),
        item("w2", "D1", "Ana Lopez", false),
    ];
    let (view, vcx) =
        cx.add_window_view(|_, cx| QuickSwitcher::new(SwitchDelegate::new(items, vec![]), cx));
    let events: Events = Rc::new(RefCell::new(Vec::new()));
    let e = events.clone();
    vcx.update(|w, cx| {
        let h = view.read(cx).focus_handle(cx);
        w.focus(&h, cx);
        cx.subscribe(&view, move |_, ev: &PickerEvent, _| {
            e.borrow_mut().push(ev.clone())
        })
        .detach();
    });
    vcx.run_until_parked();
    (view, vcx, events)
}

fn shown(view: &Entity<QuickSwitcher>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, _| {
        v.delegate()
            .matches()
            .iter()
            .map(|i| i.label.to_string())
            .collect()
    })
}

#[gpui::test]
fn typing_filters_across_workspaces(cx: &mut TestAppContext) {
    let (view, vcx, _) = setup(cx);
    assert_eq!(shown(&view, vcx).len(), 4);
    vcx.simulate_input("gen");
    assert_eq!(shown(&view, vcx), ["general", "general"]);
    vcx.simulate_keystrokes("backspace backspace backspace");
    assert_eq!(shown(&view, vcx).len(), 4);
}

#[gpui::test]
fn enter_confirms_the_selected_item(cx: &mut TestAppContext) {
    let (view, vcx, events) = setup(cx);
    vcx.simulate_input("ana");
    vcx.simulate_keystrokes("enter");
    assert_eq!(*events.borrow(), [PickerEvent::Confirmed(0)]);
    view.read_with(vcx, |v, _| {
        let c = v.delegate().chosen().expect("chosen");
        assert_eq!(c.workspace, ws("w2"));
        assert_eq!(c.target, SwitchTarget::Conversation(cid("D1")));
    });
}

#[gpui::test]
fn arrows_move_the_selection(cx: &mut TestAppContext) {
    let (view, vcx, events) = setup(cx);
    vcx.simulate_keystrokes("down down up");
    assert_eq!(view.read_with(vcx, |v, _| v.selected()), 1);
    vcx.simulate_keystrokes("up up");
    assert_eq!(view.read_with(vcx, |v, _| v.selected()), 0);
    vcx.simulate_keystrokes("enter");
    assert_eq!(*events.borrow(), [PickerEvent::Confirmed(0)]);
}

#[gpui::test]
fn escape_closes(cx: &mut TestAppContext) {
    let (_view, vcx, events) = setup(cx);
    vcx.simulate_keystrokes("escape");
    assert_eq!(*events.borrow(), [PickerEvent::Dismissed]);
}

#[gpui::test]
fn enter_with_no_match_does_nothing(cx: &mut TestAppContext) {
    let (_view, vcx, events) = setup(cx);
    vcx.simulate_input("qqqq");
    vcx.simulate_keystrokes("enter");
    assert!(events.borrow().is_empty());
}

#[gpui::test]
fn a_scoped_switcher_names_what_it_lists(cx: &mut TestAppContext) {
    use components::SwitchScope;
    cx.update(|cx| {
        i18n::gpui::init(cx, &[components::catalog()], i18n::LocalePref::System);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    let texts = |scope: SwitchScope, cx: &mut TestAppContext| {
        let d = SwitchDelegate::new(vec![], vec![]).with_scope(scope);
        assert_eq!(d.scope(), scope);
        cx.update(|cx| (d.placeholder(cx), d.empty_text(cx)))
    };
    assert_eq!(texts(SwitchScope::All, cx).0, "Jump to a conversation");
    assert_eq!(
        texts(SwitchScope::Compose, cx).0,
        "Write to a channel or person"
    );
    assert_eq!(
        texts(SwitchScope::Channels, cx),
        ("Find a channel".into(), "No matching channels.".into())
    );
    assert_eq!(
        texts(SwitchScope::People, cx),
        ("Find a person".into(), "No matching people.".into())
    );
}

#[gpui::test]
fn found_channels_join_the_matches_once_and_are_marked(cx: &mut TestAppContext) {
    let (view, vcx, _) = setup(cx);
    vcx.simulate_input("gen");
    let mut found = item("w1", "C7", "general-chat", false);
    found.joined = false;
    // Already listed: not twice.
    let dup = item("w1", "C1", "general", false);
    view.update(vcx, |v, cx| {
        v.update_delegate(cx, |d| d.set_found(vec![found.clone(), dup]))
    });
    assert_eq!(shown(&view, vcx), ["general", "general", "general-chat"]);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("switcher-not-joined-2").is_some());
    assert!(vcx.debug_bounds("switcher-not-joined-0").is_none());
    // A new search replaces what the last one found.
    view.update(vcx, |v, cx| {
        v.update_delegate(cx, |d| d.set_found(Vec::new()))
    });
    assert_eq!(shown(&view, vcx), ["general", "general"]);
}
