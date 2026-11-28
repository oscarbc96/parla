use std::cell::RefCell;
use std::rc::Rc;

use components::{Completion, CompletionKind, Composer, ComposerEvent, ComposerSource};
use domain::test_support::{cid, uid};
use domain::{
    Broadcast, Capabilities, ComposedText, ConversationId, EmojiRef, Inline, MessageWindow, UserId,
};
use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};

type Used = Rc<RefCell<Vec<EmojiRef>>>;

struct Src {
    me: UserId,
    caps: Capabilities,
    recent: Vec<EmojiRef>,
    used: Used,
}

fn emoji(code: &str, glyph: Option<&str>) -> EmojiRef {
    EmojiRef {
        shortcode: code.into(),
        unicode: glyph.map(Into::into),
        skin_tone: None,
    }
}

impl ComposerSource for Src {
    fn window(&self, _: &ConversationId) -> Option<&MessageWindow> {
        None
    }
    fn self_user(&self) -> Option<&UserId> {
        Some(&self.me)
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn completions(&self, kind: CompletionKind, query: &str) -> Vec<Completion> {
        let q = query.to_lowercase();
        let hit = |s: &str| s.to_lowercase().contains(&q);
        match kind {
            CompletionKind::Person => [("U1", "Ana Lopez"), ("U2", "Anselm"), ("U3", "Bob")]
                .into_iter()
                .filter(|(_, n)| hit(n))
                .map(|(id, n)| Completion {
                    label: format!("@{n}").into(),
                    detail: None,
                    members: None,
                    members_label: None,
                    image: None,
                    inline: Inline::User {
                        id: uid(id),
                        label: None,
                    },
                })
                .chain(hit("engineering").then(|| Completion {
                    label: "@eng".into(),
                    detail: Some("Engineering".into()),
                    members: Some(12),
                    members_label: None,
                    image: None,
                    inline: Inline::Group {
                        id: "S1".into(),
                        label: None,
                    },
                }))
                .chain(hit("here").then(|| Completion {
                    label: "@here".into(),
                    detail: None,
                    members: None,
                    members_label: None,
                    image: None,
                    inline: Inline::Broadcast(Broadcast::Here),
                }))
                .collect(),
            CompletionKind::Conversation => ["general", "random"]
                .into_iter()
                .filter(|n| hit(n))
                .map(|n| Completion {
                    label: format!("#{n}").into(),
                    detail: None,
                    members: None,
                    members_label: None,
                    image: None,
                    inline: Inline::Conversation {
                        id: cid(&format!("C-{n}")),
                        label: None,
                    },
                })
                .collect(),
            CompletionKind::Emoji => [
                emoji("tada", Some("\u{1f389}")),
                emoji("taco", Some("\u{1f32e}")),
                emoji("partyparrot", None),
            ]
            .into_iter()
            .filter(|e| hit(&e.shortcode))
            .map(|e| Completion {
                label: e
                    .unicode
                    .as_deref()
                    .map_or_else(|| format!(":{}:", e.shortcode), str::to_string)
                    .into(),
                detail: Some(format!(":{}:", e.shortcode).into()),
                members: None,
                members_label: None,
                image: e.unicode.is_none().then(|| {
                    domain::ImageRef::Web(
                        format!("https://example.invalid/{}.png", e.shortcode).into(),
                    )
                }),
                inline: Inline::Emoji(e),
            })
            .collect(),
        }
    }
    fn recent_emoji(&self) -> Vec<EmojiRef> {
        self.recent.clone()
    }
    fn emoji_used(&mut self, e: EmojiRef, _: &mut gpui::App) {
        self.used.borrow_mut().push(e);
    }
}

type Events = Rc<RefCell<Vec<ComposerEvent>>>;

fn setup(
    cx: &mut TestAppContext,
    recent: Vec<EmojiRef>,
) -> (Entity<Composer<Src>>, Events, Used, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let used = Used::default();
    let u = used.clone();
    let src = cx.new(move |_| Src {
        me: uid("u0"),
        caps: Capabilities::default(),
        recent,
        used: u,
    });
    let (view, vcx) = cx.add_window_view(move |_w, cx| Composer::new(src, cid("C1"), cx));
    let events: Events = Rc::default();
    let sink = events.clone();
    vcx.update(|w, cx| {
        cx.subscribe(&view, move |_, e: &ComposerEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach();
        let handle = view.read(cx).focus_handle(cx);
        w.focus(&handle, cx);
    });
    (view, events, used, vcx)
}

fn text(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) -> String {
    v.read_with(vcx, |c, cx| c.text(cx).to_string())
}

fn composed(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) -> ComposedText {
    v.read_with(vcx, |c, cx| c.composed(cx))
}

fn labels(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) -> Option<Vec<String>> {
    v.read_with(vcx, |c, _| {
        c.completion_items()
            .map(|i| i.iter().map(|c| c.label.to_string()).collect())
    })
}

#[gpui::test]
fn at_opens_people_and_inserts_span(cx: &mut TestAppContext) {
    let (v, ev, _, vcx) = setup(cx, vec![]);
    vcx.simulate_input("hi @an");
    assert_eq!(
        labels(&v, vcx),
        Some(vec!["@Ana Lopez".into(), "@Anselm".into()])
    );
    vcx.simulate_keystrokes("enter");
    assert!(ev.borrow().is_empty(), "enter accepted, it did not send");
    assert_eq!(text(&v, vcx), "hi @Ana Lopez ");
    let c = composed(&v, vcx);
    assert_eq!(c.spans.len(), 1);
    assert_eq!(c.spans[0].range, 3..13);
    assert!(matches!(&c.spans[0].inline, Inline::User { id, .. } if id.0.as_ref() == "U1"));
    assert_eq!(labels(&v, vcx), None, "popup closed after accepting");
}

#[gpui::test]
fn at_offers_groups_and_inserts_a_group_span(cx: &mut TestAppContext) {
    let (v, ev, _, vcx) = setup(cx, vec![]);
    vcx.simulate_input("ping @engin");
    assert_eq!(labels(&v, vcx), Some(vec!["@eng".into()]));
    let row = v.read_with(vcx, |c, _| c.completion_items().unwrap()[0].clone());
    assert_eq!(row.detail.as_deref(), Some("Engineering"));
    assert_eq!(row.members, Some(12));
    assert_eq!(
        row.members_label.as_deref(),
        Some("Group \u{b7} 12 people"),
        "the size text is built once, with the row"
    );
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&v, vcx), "ping @eng ");
    vcx.simulate_input("now");
    vcx.simulate_keystrokes("enter");
    let sent = ev.borrow();
    let [ComposerEvent::Submit { body }] = &sent[..] else {
        panic!("{sent:?}")
    };
    assert_eq!(body.text.as_ref(), "ping @eng now");
    assert_eq!(body.spans.len(), 1);
    assert_eq!(body.spans[0].range, 5..9);
    assert!(
        matches!(&body.spans[0].inline, Inline::Group { id, label: None } if id.as_ref() == "S1")
    );
}

#[gpui::test]
fn restored_text_does_not_open_a_picker(cx: &mut TestAppContext) {
    let (v, _, _, vcx) = setup(cx, vec![]);
    v.update(vcx, |c, cx| c.set_text("thanks @an".into(), cx));
    vcx.run_until_parked();
    assert_eq!(labels(&v, vcx), None, "a restored draft is not typing");
    v.update(vcx, |c, cx| {
        c.begin_edit(
            domain::test_support::mid("C1", "1.000001"),
            "see #ge".into(),
            cx,
        )
    });
    vcx.run_until_parked();
    assert_eq!(labels(&v, vcx), None, "nor is the text of an edit");
    // Typing a new trigger afterwards works.
    vcx.simulate_input(" #rand");
    assert_eq!(labels(&v, vcx), Some(vec!["#random".into()]));
}

#[gpui::test]
fn arrows_and_tab_pick_another_row(cx: &mut TestAppContext) {
    let (v, _, _, vcx) = setup(cx, vec![]);
    vcx.simulate_input("@an");
    vcx.simulate_keystrokes("down");
    vcx.simulate_keystrokes("up");
    vcx.simulate_keystrokes("down");
    vcx.simulate_keystrokes("tab");
    assert_eq!(text(&v, vcx), "@Anselm ");
    assert_eq!(composed(&v, vcx).spans.len(), 1);
}

#[gpui::test]
fn hash_opens_conversations(cx: &mut TestAppContext) {
    let (v, _, _, vcx) = setup(cx, vec![]);
    vcx.simulate_input("see #ge");
    assert_eq!(labels(&v, vcx), Some(vec!["#general".into()]));
    vcx.simulate_keystrokes("tab");
    let c = composed(&v, vcx);
    assert_eq!(c.text.as_ref(), "see #general ");
    assert!(
        matches!(&c.spans[0].inline, Inline::Conversation { id, .. } if id.0.as_ref() == "C-general")
    );
}

#[gpui::test]
fn colon_needs_two_chars_recent_first(cx: &mut TestAppContext) {
    let (v, _, used, vcx) = setup(cx, vec![emoji("taco", Some("\u{1f32e}"))]);
    vcx.simulate_input(":t");
    assert_eq!(labels(&v, vcx), None, "one character is not enough");
    vcx.simulate_input("a");
    assert_eq!(
        labels(&v, vcx),
        Some(vec!["\u{1f32e}".into(), "\u{1f389}".into()]),
        "recent first"
    );
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&v, vcx), "\u{1f32e} ");
    assert!(composed(&v, vcx).spans.is_empty(), "a glyph is plain text");
    assert_eq!(*used.borrow(), [emoji("taco", Some("\u{1f32e}"))]);
}

#[gpui::test]
fn custom_emoji_is_inserted_as_code_span(cx: &mut TestAppContext) {
    let (v, _, used, vcx) = setup(cx, vec![]);
    vcx.simulate_input("x :part");
    vcx.simulate_keystrokes("enter");
    let c = composed(&v, vcx);
    assert_eq!(c.text.as_ref(), "x :partyparrot: ");
    assert_eq!(c.spans.len(), 1);
    assert_eq!(c.spans[0].range, 2..15);
    assert_eq!(*used.borrow(), [emoji("partyparrot", None)]);
}

#[gpui::test]
fn trigger_only_at_word_start(cx: &mut TestAppContext) {
    let (v, _, _, vcx) = setup(cx, vec![]);
    vcx.simulate_input("a@b");
    assert_eq!(labels(&v, vcx), None);
    vcx.simulate_input(" ");
    vcx.simulate_input("mail#ge");
    assert_eq!(labels(&v, vcx), None);
    vcx.simulate_input(" @b");
    assert_eq!(labels(&v, vcx), Some(vec!["@Bob".into()]));
}

#[gpui::test]
fn escape_keeps_text(cx: &mut TestAppContext) {
    let (v, ev, _, vcx) = setup(cx, vec![]);
    vcx.simulate_input("hi @an");
    vcx.simulate_keystrokes("escape");
    assert_eq!(labels(&v, vcx), None);
    assert_eq!(text(&v, vcx), "hi @an");
    assert!(ev.borrow().is_empty(), "escape only dismissed the popup");
    vcx.simulate_input("a");
    assert_eq!(labels(&v, vcx), None, "stays dismissed for this word");
}

#[gpui::test]
fn backspace_deletes_whole_span(cx: &mut TestAppContext) {
    let (v, _, _, vcx) = setup(cx, vec![]);
    vcx.simulate_input("hi @an");
    vcx.simulate_keystrokes("enter");
    vcx.simulate_keystrokes("backspace");
    assert_eq!(text(&v, vcx), "hi @Ana Lopez");
    assert_eq!(
        labels(&v, vcx),
        None,
        "a picked span never reopens the popup"
    );
    vcx.simulate_keystrokes("backspace");
    assert_eq!(text(&v, vcx), "hi ");
    assert!(composed(&v, vcx).spans.is_empty());
}

#[gpui::test]
fn typed_at_is_literal_text(cx: &mut TestAppContext) {
    let (v, ev, _, vcx) = setup(cx, vec![]);
    vcx.simulate_input("@ana");
    vcx.simulate_keystrokes("escape");
    assert!(composed(&v, vcx).spans.is_empty());
    vcx.simulate_keystrokes("enter");
    let sent = ev.borrow();
    let [ComposerEvent::Submit { body }] = &sent[..] else {
        panic!("{sent:?}")
    };
    assert_eq!(body.text.as_ref(), "@ana");
    assert!(body.spans.is_empty());
}

#[gpui::test]
fn submit_carries_trimmed_text_with_shifted_spans(cx: &mut TestAppContext) {
    let (_, ev, _, vcx) = setup(cx, vec![]);
    vcx.simulate_input("  @bo");
    vcx.simulate_keystrokes("enter");
    vcx.simulate_input("x");
    vcx.simulate_keystrokes("enter");
    let sent = ev.borrow();
    let [ComposerEvent::Submit { body }] = &sent[..] else {
        panic!("{sent:?}")
    };
    assert_eq!(body.text.as_ref(), "@Bob x");
    assert_eq!(body.spans.len(), 1);
    assert_eq!(body.spans[0].range, 0..4);
}

mod data {
    use components::completion::{conversations, emoji, people};
    use domain::test_support::{conv, member};
    use domain::{ConversationKind, CustomEmoji, ImageRef, WorkspaceState};
    use settings::WorkspaceKey;

    use super::*;

    fn state() -> WorkspaceState {
        let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 100);
        for (id, name) in [("U1", "Ana Lopez"), ("U2", "Bob"), ("U3", "Anselm")] {
            s.members.insert(uid(id), member(id, name));
        }
        let mut gone = member("U4", "Anabel");
        gone.deleted = true;
        s.members.insert(uid("U4"), gone);
        let mut handled = member("U5", "Zed");
        handled.real_name = "Zed Zimmer".into();
        handled.handle = "zz".into();
        s.members.insert(uid("U5"), handled);
        s.conversations.insert(cid("C1"), conv("C1", "general"));
        s.conversations.insert(cid("C2"), conv("C2", "random"));
        let mut old = conv("C3", "general-old");
        old.is_archived = true;
        s.conversations.insert(cid("C3"), old);
        let mut dm = conv("D1", "ana");
        dm.kind = ConversationKind::Direct;
        s.conversations.insert(cid("D1"), dm);
        s
    }

    fn labels(v: Vec<Completion>) -> Vec<String> {
        v.into_iter().map(|c| c.label.to_string()).collect()
    }

    #[test]
    fn people_match_names_skip_deleted_and_add_broadcasts() {
        let s = state();
        let got = labels(people(&s, Some(&cid("C1")), "an"));
        assert_eq!(got, ["@Ana Lopez", "@Anselm"]);
        let all = labels(people(&s, Some(&cid("C1")), ""));
        assert_eq!(&all[..3], ["@here", "@channel", "@everyone"]);
        assert!(
            !all.contains(&"@Anabel".to_string()),
            "deleted users are skipped"
        );
        let h = labels(people(&s, Some(&cid("C1")), "chan"));
        assert_eq!(h, ["@channel"]);
    }

    fn group(id: &str, handle: &str, name: &str, n: u32) -> domain::UserGroup {
        domain::UserGroup {
            id: id.into(),
            handle: handle.into(),
            name: name.into(),
            member_count: n,
            is_member: false,
        }
    }

    #[test]
    fn groups_match_handle_and_name_and_carry_their_size() {
        let mut s = state();
        for g in [
            group("S1", "eng", "Engineering", 12),
            group("S2", "design", "Design crew", 4),
        ] {
            s.user_groups.insert(g.id.clone(), g);
        }
        let got = people(&s, Some(&cid("C1")), "eng");
        assert_eq!(labels(got.clone()), ["@eng"]);
        assert_eq!(got[0].detail.as_deref(), Some("Engineering"));
        assert_eq!(got[0].members, Some(12));
        assert!(matches!(&got[0].inline, Inline::Group { id, label: None } if id.as_ref() == "S1"));
        assert_eq!(labels(people(&s, None, "crew")), ["@design"]);
        let all = labels(people(&s, Some(&cid("C1")), ""));
        assert!(all.contains(&"@eng".to_string()) && all.contains(&"@design".to_string()));
        assert_eq!(
            &all[..3],
            ["@here", "@channel", "@everyone"],
            "broadcasts still lead"
        );
    }

    #[test]
    fn broadcasts_are_not_offered_in_direct_messages() {
        let s = state();
        let all = labels(people(&s, Some(&cid("D1")), ""));
        assert!(all.iter().all(|l| !l.starts_with("@here")));
        assert!(all.contains(&"@Bob".to_string()));
    }

    #[test]
    fn people_match_handle_and_real_name() {
        let s = state();
        assert_eq!(labels(people(&s, None, "zz")), ["@Zed"]);
        assert_eq!(labels(people(&s, None, "zimm")), ["@Zed"]);
    }

    #[test]
    fn conversations_are_channels_only_and_not_archived() {
        let s = state();
        assert_eq!(labels(conversations(&s, "")), ["#general", "#random"]);
        assert_eq!(labels(conversations(&s, "gen")), ["#general"]);
        assert!(
            conversations(&s, "ana").is_empty(),
            "direct messages are not channels"
        );
    }

    #[test]
    fn emoji_lists_standard_and_custom() {
        let custom = [CustomEmoji {
            shortcode: "partyparrot".into(),
            image: ImageRef::Web("https://example.invalid/p.png".into()),
        }];
        let got = emoji(&custom, "tada");
        assert_eq!(got[0].label.as_ref(), "\u{1f389}");
        assert_eq!(got[0].detail.as_deref(), Some(":tada:"));
        let got = emoji(&custom, "party");
        let custom_row = got
            .iter()
            .find(|c| c.label.as_ref() == ":partyparrot:")
            .expect("custom emoji is listed");
        assert!(matches!(&custom_row.inline, Inline::Emoji(e) if e.unicode.is_none()));
        assert!(got.len() <= components::completion::SOURCE_LIMIT);
    }

    #[test]
    fn emoji_rows_carry_their_image() {
        let custom = [CustomEmoji {
            shortcode: "partyparrot".into(),
            image: ImageRef::Web("https://example.invalid/p.png".into()),
        }];
        let got = emoji(&custom, "party");
        let row = got
            .iter()
            .find(|c| c.label.as_ref() == ":partyparrot:")
            .expect("custom emoji is listed");
        assert_eq!(
            row.image,
            Some(ImageRef::Web("https://example.invalid/p.png".into()))
        );
        let standard = emoji(&custom, "tada");
        assert_eq!(standard[0].label.as_ref(), "\u{1f389}");
        assert!(standard[0].image.is_none());
    }
}
