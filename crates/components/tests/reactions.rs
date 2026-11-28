mod common;

use std::sync::Arc;

use components::{
    AddReaction, EmojiPicked, EmojiPicker, PickerTab, Timeline, TimelineSource, pill_label,
    tooltip_text,
};
use domain::test_support::{cid, member, msg, uid};
use domain::{
    Capabilities, ConversationId, CustomEmoji, EmojiRef, HistoryPage, ImageRef, Input, Member,
    Message, MessageId, MessageWindow, Reaction, UserId, WorkspaceState, reduce,
};
use gpui::{App, AppContext as _, Entity, TestAppContext, VisualContext as _};
use settings::{Clock, WorkspaceKey};

fn thumbs() -> EmojiRef {
    EmojiRef {
        shortcode: "thumbsup".into(),
        unicode: Some("\u{1F44D}".into()),
        skin_tone: None,
    }
}

struct Src {
    state: WorkspaceState,
    caps: Capabilities,
    custom: Vec<CustomEmoji>,
    reacts: Vec<(MessageId, EmojiRef, bool)>,
}

impl TimelineSource for Src {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn self_user(&self) -> Option<&UserId> {
        None
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn load_older(&mut self, _: ConversationId, _: &mut App) {}
    fn edit(&mut self, _: MessageId, _: String, _: &mut App) {}
    fn delete(&mut self, _: MessageId, _: &mut App) {}
    fn react(&mut self, id: MessageId, emoji: EmojiRef, add: bool, _: &mut App) {
        self.reacts.push((id, emoji, add));
    }
    fn custom_emoji(&self) -> Vec<CustomEmoji> {
        self.custom.clone()
    }
}

fn setup(
    cx: &mut TestAppContext,
    caps: Capabilities,
) -> (
    Entity<Src>,
    Entity<Timeline<Src>>,
    &mut gpui::VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    state.members.insert(uid("u2"), member("u2", "Bob"));
    let mut m = msg("C1", "1000.000001", "hi");
    m.reactions = Arc::from(vec![Reaction {
        emoji: thumbs(),
        count: 2,
        users: Arc::from(vec![uid("u1"), uid("u2")]),
        me: true,
    }]);
    reduce(
        &mut state,
        Input::HistoryLoaded {
            conversation: cid("C1"),
            page: HistoryPage {
                messages: vec![m],
                has_more: false,
            },
        },
    );
    let src = cx.new(|_| Src {
        state,
        caps,
        custom: vec![CustomEmoji {
            shortcode: "partyparrot".into(),
            image: ImageRef::Web("https://example.invalid/p.png".into()),
        }],
        reacts: vec![],
    });
    let s2 = src.clone();
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(s2, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    (src, view, vcx)
}

fn id() -> MessageId {
    domain::test_support::mid("C1", "1000.000001")
}

#[gpui::test]
fn pills_show_counts_and_me(cx: &mut TestAppContext) {
    let (_s, view, vcx) = setup(cx, Capabilities::default());
    let pills = view.read_with(vcx, |v, cx| v.reaction_pills(&id(), cx));
    assert_eq!(pills.len(), 1);
    assert_eq!(pills[0].label, "\u{1F44D} 2");
    assert!(pills[0].me);
    assert_eq!(
        pill_label(&Reaction {
            emoji: EmojiRef {
                shortcode: "partyparrot".into(),
                unicode: None,
                skin_tone: None
            },
            count: 3,
            users: Arc::from(vec![]),
            me: false,
        }),
        ":partyparrot: 3"
    );
}

#[gpui::test]
fn click_toggles_reaction(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, Capabilities::default());
    view.update(vcx, |v, cx| v.toggle_reaction(id(), thumbs(), cx));
    assert_eq!(
        src.read_with(vcx, |s, _| s.reacts.clone()),
        vec![(id(), thumbs(), false)]
    );
}

#[gpui::test]
fn picker_fuzzy_finds_shortcodes(cx: &mut TestAppContext) {
    let (_s, _v, vcx) = setup(cx, Capabilities::default());
    let p = vcx.new(|cx| EmojiPicker::new(vec![], false, cx));
    p.update(vcx, |p, cx| p.set_query("thup".into(), cx));
    let first = p.read_with(vcx, |p, _| p.results().into_iter().next());
    assert_eq!(
        first.map(|e| e.shortcode.to_string()),
        Some("thumbsup".into())
    );
}

#[gpui::test]
fn picker_recent_row_reads_the_workspace_list(cx: &mut TestAppContext) {
    let (_s, _v, vcx) = setup(cx, Capabilities::default());
    let e = |c: &str, g: &str| domain::EmojiRef {
        shortcode: c.into(),
        unicode: Some(g.into()),
        skin_tone: None,
    };
    let p = vcx.new(|cx| EmojiPicker::new(vec![], false, cx));
    assert!(p.read_with(vcx, |p, _| p.recent()).is_empty());
    p.update(vcx, |p, cx| {
        p.set_recent(vec![e("taco", "\u{1f32e}"), e("tada", "\u{1f389}")], cx)
    });
    let got = p.read_with(vcx, |p, _| p.recent());
    assert_eq!(
        got.iter().map(|e| &*e.shortcode).collect::<Vec<_>>(),
        ["taco", "tada"]
    );
    // Picking moves it to the front.
    p.update(vcx, |p, cx| {
        p.set_query("tada".into(), cx);
        p.confirm(cx)
    });
    let got = p.read_with(vcx, |p, _| p.recent());
    assert_eq!(
        got.iter().map(|e| &*e.shortcode).collect::<Vec<_>>(),
        ["tada", "taco"]
    );
}

#[gpui::test]
fn picker_recent_row_emoji_can_be_clicked(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let e = |c: &str, g: &str| EmojiRef {
        shortcode: c.into(),
        unicode: Some(g.into()),
        skin_tone: None,
    };
    let (p, vcx) = cx.add_window_view(|_, cx| {
        let mut p = EmojiPicker::new(vec![], false, cx);
        p.set_recent(vec![e("taco", "\u{1f32e}"), e("tada", "\u{1f389}")], cx);
        p
    });
    let picked: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let got = picked.clone();
    let sub = vcx.update(|_, cx| {
        cx.subscribe(&p, move |_, ev: &EmojiPicked, _| {
            got.lock().unwrap().push(ev.0.shortcode.to_string())
        })
    });
    std::mem::forget(sub);
    let b = vcx
        .debug_bounds("picker-recent-1")
        .expect("recent cell drawn");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(*picked.lock().unwrap(), ["tada"]);
}

#[gpui::test]
fn picker_keyboard_reaches_the_recent_row(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let e = |c: &str, g: &str| EmojiRef {
        shortcode: c.into(),
        unicode: Some(g.into()),
        skin_tone: None,
    };
    let (p, vcx) = cx.add_window_view(|_, cx| {
        let mut p = EmojiPicker::new(vec![], false, cx);
        p.set_recent(vec![e("taco", "\u{1f32e}"), e("tada", "\u{1f389}")], cx);
        p
    });
    vcx.update(|w, cx| {
        let f = gpui::Focusable::focus_handle(p.read(cx), cx);
        w.focus(&f, cx);
    });
    let picked: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let got = picked.clone();
    let sub = vcx.update(|_, cx| {
        cx.subscribe(&p, move |_, ev: &EmojiPicked, _| {
            got.lock().unwrap().push(ev.0.shortcode.to_string())
        })
    });
    std::mem::forget(sub);
    assert_eq!(p.read_with(vcx, |p, _| p.recent_selected()), None);
    // Up from the grid's first row enters Recent; right walks it.
    vcx.simulate_keystrokes("up");
    assert_eq!(p.read_with(vcx, |p, _| p.recent_selected()), Some(0));
    vcx.simulate_keystrokes("right");
    assert_eq!(p.read_with(vcx, |p, _| p.recent_selected()), Some(1));
    vcx.simulate_keystrokes("enter");
    assert_eq!(*picked.lock().unwrap(), ["tada"]);
    // Down returns to the grid.
    vcx.simulate_keystrokes("down");
    assert_eq!(p.read_with(vcx, |p, _| p.recent_selected()), None);
}

#[gpui::test]
fn custom_tab_needs_capability(cx: &mut TestAppContext) {
    let (_s, _v, vcx) = setup(cx, Capabilities::default());
    let custom = vec![CustomEmoji {
        shortcode: "partyparrot".into(),
        image: ImageRef::Web("https://example.invalid/p.png".into()),
    }];
    let off = vcx.new(|cx| EmojiPicker::new(custom.clone(), false, cx));
    assert_eq!(
        off.read_with(vcx, |p, _| p.tabs()),
        vec![PickerTab::Standard]
    );
    let on = vcx.new(|cx| EmojiPicker::new(custom, true, cx));
    assert_eq!(
        on.read_with(vcx, |p, _| p.tabs()),
        vec![PickerTab::Standard, PickerTab::Custom]
    );
    on.update(vcx, |p, cx| p.select_tab(PickerTab::Custom, cx));
    let r = on.read_with(vcx, |p, _| p.results());
    assert_eq!(r.len(), 1);
    assert_eq!(&*r[0].shortcode, "partyparrot");
    assert!(r[0].unicode.is_none());
}

#[gpui::test]
fn custom_tab_is_sorted_by_shortcode(cx: &mut TestAppContext) {
    let (_s, _v, vcx) = setup(cx, Capabilities::default());
    let custom = |codes: &[&str]| -> Vec<CustomEmoji> {
        codes
            .iter()
            .map(|c| CustomEmoji {
                shortcode: (*c).into(),
                image: ImageRef::Web(format!("https://example.invalid/{c}.png").into()),
            })
            .collect()
    };
    let p = vcx.new(|cx| EmojiPicker::new(custom(&["zeta", "alpha", "mike"]), true, cx));
    p.update(vcx, |p, cx| p.select_tab(PickerTab::Custom, cx));
    let got = p.read_with(vcx, |p, _| p.results());
    assert_eq!(
        got.iter().map(|e| &*e.shortcode).collect::<Vec<_>>(),
        ["alpha", "mike", "zeta"]
    );
}

#[gpui::test]
fn emoji_tab_search_finds_custom_emoji_when_enabled(cx: &mut TestAppContext) {
    let (_s, _v, vcx) = setup(cx, Capabilities::default());
    let custom = vec![CustomEmoji {
        shortcode: "partyparrot".into(),
        image: ImageRef::Web("https://example.invalid/p.png".into()),
    }];
    let on = vcx.new(|cx| EmojiPicker::new(custom.clone(), true, cx));
    on.update(vcx, |p, cx| p.set_query("party".into(), cx));
    let got = on.read_with(vcx, |p, _| p.results());
    assert!(
        got.iter()
            .any(|e| e.unicode.is_none() && &*e.shortcode == "partyparrot"),
        "the Emoji tab merges custom emoji: {got:?}"
    );
    let off = vcx.new(|cx| EmojiPicker::new(custom, false, cx));
    off.update(vcx, |p, cx| p.set_query("party".into(), cx));
    let got = off.read_with(vcx, |p, _| p.results());
    assert!(
        got.iter().all(|e| &*e.shortcode != "partyparrot"),
        "without the capability custom emoji stay out: {got:?}"
    );
}

#[gpui::test]
fn custom_tab_lists_more_than_the_standard_cap(cx: &mut TestAppContext) {
    let (_s, _v, vcx) = setup(cx, Capabilities::default());
    let custom: Vec<CustomEmoji> = (0..500)
        .map(|i| CustomEmoji {
            shortcode: format!("custom{i:03}").into(),
            image: ImageRef::Web(format!("https://example.invalid/{i}.png").into()),
        })
        .collect();
    let p = vcx.new(|cx| EmojiPicker::new(custom, true, cx));
    p.update(vcx, |p, cx| p.select_tab(PickerTab::Custom, cx));
    let got = p.read_with(vcx, |p, _| p.results());
    assert_eq!(got.len(), 500);
    assert_eq!(&*got[0].shortcode, "custom000");
    assert_eq!(&*got[499].shortcode, "custom499");
}

#[gpui::test]
fn keyboard_selection_advances_by_a_row_in_a_long_custom_tab(cx: &mut TestAppContext) {
    let (_s, _v, _vcx) = setup(cx, Capabilities::default());
    let custom: Vec<CustomEmoji> = (0..500)
        .map(|i| CustomEmoji {
            shortcode: format!("custom{i:03}").into(),
            image: ImageRef::Web(format!("https://example.invalid/{i}.png").into()),
        })
        .collect();
    let (p, vcx) = cx.add_window_view(|_, cx| {
        let mut p = EmojiPicker::new(custom, true, cx);
        p.select_tab(PickerTab::Custom, cx);
        p
    });
    vcx.update(|w, cx| {
        let f = gpui::Focusable::focus_handle(p.read(cx), cx);
        w.focus(&f, cx);
    });
    let picked: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let got = picked.clone();
    let sub = vcx.update(|_, cx| {
        cx.subscribe(&p, move |_, ev: &EmojiPicked, _| {
            got.lock().unwrap().push(ev.0.shortcode.to_string())
        })
    });
    std::mem::forget(sub);
    // Down moves a whole row; ten rows land on `custom080`.
    for _ in 0..10 {
        vcx.simulate_keystrokes("down");
    }
    vcx.simulate_keystrokes("enter");
    assert_eq!(*picked.lock().unwrap(), ["custom080"]);
}

#[gpui::test]
fn add_reaction_action_opens_picker_and_reacts(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, Capabilities::default());
    view.update(vcx, |v, cx| v.select(Some(id()), cx));
    vcx.run_until_parked();
    vcx.focus(&view);
    vcx.dispatch_action(AddReaction);
    let picker = view
        .read_with(vcx, |v, _| v.picker().cloned())
        .expect("picker opened");
    picker.update(vcx, |p, cx| p.set_query("thup".into(), cx));
    let got = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let g2 = got.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&picker, move |_, ev: &EmojiPicked, _| {
            g2.borrow_mut().push(ev.0.clone())
        })
        .detach()
    });
    picker.update(vcx, |p, cx| p.confirm(cx));
    vcx.run_until_parked();
    assert_eq!(got.borrow().len(), 1);
    let reacts = src.read_with(vcx, |s, _| s.reacts.clone());
    assert_eq!(reacts.len(), 1);
    assert_eq!(reacts[0].0, id());
    assert_eq!(&*reacts[0].1.shortcode, "thumbsup");
    assert!(reacts[0].2);
    assert!(
        view.read_with(vcx, |v, _| v.picker().is_none()),
        "closed after pick"
    );
}

#[gpui::test]
fn tooltip_caps_names(cx: &mut TestAppContext) {
    let _ = cx;
    let names: Vec<String> = (0..15).map(|i| format!("user{i}")).collect();
    let t = tooltip_text(&names, &common::en());
    assert!(t.contains("user9"));
    assert!(!t.contains("user10"));
    assert!(t.ends_with("and 5 others"), "{t}");
    assert_eq!(tooltip_text(&names[..2], &common::en()), "user0, user1");
    let _: Message = msg("C1", "1.000000", "x");
}

#[gpui::test]
fn switching_conversation_closes_the_picker(cx: &mut TestAppContext) {
    let (_s, view, vcx) = setup(cx, Capabilities::default());
    vcx.update(|w, cx| view.update(cx, |t, cx| t.open_picker(id(), w, cx)));
    view.read_with(vcx, |t, _| assert!(t.picker().is_some()));
    view.update(vcx, |t, cx| {
        t.set_conversation(domain::test_support::cid("C2"), cx)
    });
    view.read_with(vcx, |t, _| assert!(t.picker().is_none()));
}
