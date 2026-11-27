//! The "is typing" line above the composer and the typing signal it sends.

use std::cell::RefCell;
use std::rc::Rc;

use components::{Composer, ComposerSource};
use domain::test_support::{cid, member, mid, uid};
use domain::{Capabilities, ConversationId, Member, MessageTs, MessageWindow, UserId};
use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};

type Sent = Rc<RefCell<Vec<(ConversationId, Option<MessageTs>)>>>;

struct Src {
    me: UserId,
    caps: Capabilities,
    members: Vec<Member>,
    /// Who types in the conversation, and in the thread `5.000000`.
    main: Vec<UserId>,
    thread: Vec<UserId>,
    sent: Sent,
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
    fn typed(&mut self, c: ConversationId, thread: Option<MessageTs>, _: &mut gpui::App) {
        self.sent.borrow_mut().push((c, thread));
    }
    fn typing(&self, _: &ConversationId, thread: Option<&MessageTs>) -> Vec<Option<&Member>> {
        let who = if thread.is_some() {
            &self.thread
        } else {
            &self.main
        };
        who.iter()
            .map(|u| self.members.iter().find(|m| &m.id == u))
            .collect()
    }
}

fn setup<'a>(
    cx: &'a mut TestAppContext,
    main: &[&str],
    thread: &[&str],
) -> (Entity<Composer<Src>>, Sent, &'a mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let sent: Sent = Rc::default();
    let src = cx.new(|_| Src {
        me: uid("u1"),
        caps: Capabilities {
            typing: true,
            ..Default::default()
        },
        members: vec![
            member("u2", "Aiko"),
            member("u3", "Ben"),
            member("u4", "Chen"),
        ],
        main: main.iter().map(|u| uid(u)).collect(),
        thread: thread.iter().map(|u| uid(u)).collect(),
        sent: sent.clone(),
    });
    let (view, vcx) = cx.add_window_view(move |_w, cx| Composer::new(src, cid("C1"), cx));
    vcx.update(|w, cx| {
        let h = view.read(cx).focus_handle(cx);
        w.focus(&h, cx);
    });
    vcx.run_until_parked();
    (view, sent, vcx)
}

fn line(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) -> Option<String> {
    v.read_with(vcx, |c, cx| c.typing_text(cx))
}

#[gpui::test]
fn nobody_typing_draws_no_line(cx: &mut TestAppContext) {
    let (v, _, vcx) = setup(cx, &[], &[]);
    assert_eq!(line(&v, vcx), None);
    assert!(vcx.debug_bounds("composer-typing").is_none());
}

#[gpui::test]
fn one_typer_is_named(cx: &mut TestAppContext) {
    let (v, _, vcx) = setup(cx, &["u2"], &[]);
    assert_eq!(line(&v, vcx).as_deref(), Some("Aiko is typing"));
    assert!(vcx.debug_bounds("composer-typing").is_some());
}

#[gpui::test]
fn two_typers_are_both_named(cx: &mut TestAppContext) {
    let (v, _, vcx) = setup(cx, &["u2", "u3"], &[]);
    assert_eq!(line(&v, vcx).as_deref(), Some("Aiko and Ben are typing"));
}

#[gpui::test]
fn three_or_more_typers_are_several_people(cx: &mut TestAppContext) {
    let (v, _, vcx) = setup(cx, &["u2", "u3", "u4"], &[]);
    assert_eq!(line(&v, vcx).as_deref(), Some("Several people are typing"));
    assert!(vcx.debug_bounds("composer-typing").is_some());
}

#[gpui::test]
fn a_reply_composer_shows_its_threads_typers(cx: &mut TestAppContext) {
    let (v, _, vcx) = setup(cx, &["u2"], &["u3"]);
    v.update(vcx, |c, cx| {
        c.set_reply_root(Some(mid("C1", "5.000000")), cx)
    });
    vcx.run_until_parked();
    assert_eq!(line(&v, vcx).as_deref(), Some("Ben is typing"));
}

#[gpui::test]
fn typing_signals_the_conversation_and_thread(cx: &mut TestAppContext) {
    let (v, sent, vcx) = setup(cx, &[], &[]);
    vcx.simulate_input("hi");
    assert_eq!(
        sent.borrow().len(),
        2,
        "every edit is offered; the session throttles"
    );
    assert_eq!(sent.borrow()[0], (cid("C1"), None));
    v.update(vcx, |c, cx| {
        c.set_reply_root(Some(mid("C1", "5.000000")), cx)
    });
    sent.borrow_mut().clear();
    vcx.simulate_input("!");
    assert_eq!(
        sent.borrow().as_slice(),
        [(cid("C1"), Some(MessageTs("5.000000".into())))]
    );
}

#[gpui::test]
fn a_restored_draft_an_edit_or_a_cleared_draft_is_not_typing(cx: &mut TestAppContext) {
    let (v, sent, vcx) = setup(cx, &[], &[]);
    v.update(vcx, |c, cx| c.set_text("restored draft".into(), cx));
    vcx.run_until_parked();
    assert!(sent.borrow().is_empty(), "a restored draft sends nothing");
    v.update(vcx, |c, cx| {
        c.begin_edit(mid("C1", "1.000000"), "sent text".into(), cx)
    });
    vcx.simulate_input("x");
    assert!(
        sent.borrow().is_empty(),
        "editing a sent message sends nothing"
    );
    v.update(vcx, |c, cx| c.set_text(String::new(), cx));
    vcx.simulate_keystrokes("escape");
    vcx.simulate_input(" ");
    assert!(sent.borrow().is_empty(), "blank text is not typing");
}
