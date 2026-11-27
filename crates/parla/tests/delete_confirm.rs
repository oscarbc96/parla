//! Deleting a message asks first in a modal, in the main window and in
//! pop-outs: Delete runs the delete, Escape, Cancel and a click outside keep
//! the message and give focus back to the timeline.

mod support;

use std::sync::Arc;

use components::{DeleteMessage, Timeline};
use domain::ChatProvider as _;
use domain::test_support::{Call, FakeProvider, cid, conv, member, mid, uid};
use domain::{Capabilities, ContentParser, ConversationKind, Message, RichText};
use gpui::{
    AppContext as _, Entity, Focusable as _, Keystroke, TestAppContext, VisualTestContext, point,
    px,
};
use parla::root_view::{Phase, RootView};
use parla::session::WorkspaceSession;
use parla::window_state::PopOutTarget;
use parla::workspaces::WorkspacesHandle;
use support::*;

/// Markup where `<@u2>` is a mention: the plain text reads `@Other`.
struct Markup;

impl Markup {
    fn plain(text: &str) -> String {
        text.replace("<@u2>", "@Other")
    }
}

impl ContentParser for Markup {
    fn parse(&self, m: &Message) -> RichText {
        RichText::plain(&Self::plain(&m.text))
    }

    fn parse_text(&self, text: &str) -> RichText {
        RichText::plain(&Self::plain(text))
    }
}

fn mine() -> domain::MessageId {
    mid("D1", "2.000000")
}

struct Rig {
    env: Env,
    view: Entity<RootView>,
    session: Entity<WorkspaceSession>,
}

fn start(cx: &mut TestAppContext) -> (Rig, &mut VisualTestContext) {
    let p = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
            delete: true,
            edit: true,
            ..Default::default()
        }),
    );
    let mut dm = conv("D1", "dm");
    dm.kind = ConversationKind::Direct;
    p.add_conversation(dm);
    p.add_member(member("u1", "Me"));
    p.add_member(member("u2", "Other"));
    p.add_message(Message::new(
        mine(),
        Some(uid("u1")),
        Arc::from("hello <@u2>"),
    ));
    let e = env_with_parser(cx, "{}", vec![p], Some(Arc::new(Markup)));
    let dir = e.dir.path().to_path_buf();
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
        cx.set_global(parla::removal::AppDirs {
            data: dir.join("data"),
            cache: dir.join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);
    view.read_with(cxv, |v, _| assert_eq!(*v.phase(), Phase::Ready));
    let session = view.read_with(cxv, |v, _| v.session().unwrap().clone());
    e.providers[0].push_event(domain::ProviderEvent::Connected { reconnect: false });
    session.update(cxv, |s, cx| s.open(cid("D1"), cx));
    settle_until(cxv, |cx| {
        session.read_with(cx, |s, _| {
            s.state()
                .windows
                .get(&cid("D1"))
                .is_some_and(|w| w.messages().iter().any(|m| m.id == mine()))
        })
    });
    settle(cxv);
    (
        Rig {
            env: e,
            view,
            session,
        },
        cxv,
    )
}

fn deletes(r: &Rig) -> usize {
    r.env.providers[0]
        .calls()
        .iter()
        .filter(|c| matches!(c, Call::Delete(_)))
        .count()
}

fn still_there(r: &Rig, cx: &mut VisualTestContext) -> bool {
    r.session.read_with(cx, |s, _| {
        s.state().windows[&cid("D1")]
            .messages()
            .iter()
            .any(|m| m.id == mine())
    })
}

/// Asks to delete the message through the Delete action on the timeline.
fn ask(cx: &mut VisualTestContext, tl: &Entity<Timeline<WorkspaceSession>>) {
    cx.update(|w, cx| w.focus(&tl.read(cx).focus_handle(cx), cx));
    tl.update(cx, |t, cx| t.select(Some(mine()), cx));
    cx.dispatch_action(DeleteMessage);
    cx.run_until_parked();
}

fn main_timeline(r: &Rig, cx: &mut VisualTestContext) -> Entity<Timeline<WorkspaceSession>> {
    r.view.read_with(cx, |v, _| v.timeline().unwrap().clone())
}

fn timeline_focused(cx: &mut VisualTestContext, tl: &Entity<Timeline<WorkspaceSession>>) -> bool {
    cx.update(|w, cx| tl.read(cx).focus_handle(cx).is_focused(w))
}

fn open_dialog(r: &Rig, cx: &mut VisualTestContext) -> bool {
    r.view.read_with(cx, |v, _| v.delete_confirm().is_some())
}

fn click(cx: &mut VisualTestContext, sel: &'static str) {
    for _ in 0..2 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
    let b = cx
        .debug_bounds(sel)
        .unwrap_or_else(|| panic!("{sel} is drawn"));
    cx.simulate_click(b.center(), gpui::Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
async fn delete_confirms_and_shows_a_plain_preview(cx: &mut TestAppContext) {
    let (r, cx) = start(cx);
    let tl = main_timeline(&r, cx);
    ask(cx, &tl);
    assert!(open_dialog(&r, cx));
    assert_eq!(deletes(&r), 0, "nothing is deleted before the answer");
    let preview = r.view.read_with(cx, |v, cx| {
        v.delete_confirm()
            .unwrap()
            .dialog()
            .read(cx)
            .preview_str()
            .map(str::to_owned)
    });
    assert_eq!(preview.as_deref(), Some("hello @Other"));
    cx.simulate_keystrokes("enter");
    settle(cx);
    assert!(!open_dialog(&r, cx));
    assert_eq!(deletes(&r), 1);
    assert!(timeline_focused(cx, &tl), "focus returns to the timeline");
}

#[gpui::test]
async fn escape_keeps_the_message(cx: &mut TestAppContext) {
    let (r, cx) = start(cx);
    let tl = main_timeline(&r, cx);
    ask(cx, &tl);
    cx.simulate_keystrokes("escape");
    settle(cx);
    assert!(!open_dialog(&r, cx));
    assert_eq!(deletes(&r), 0);
    assert!(still_there(&r, cx));
    assert!(timeline_focused(cx, &tl));
}

#[gpui::test]
async fn cancel_keeps_the_message(cx: &mut TestAppContext) {
    let (r, cx) = start(cx);
    let tl = main_timeline(&r, cx);
    ask(cx, &tl);
    click(cx, "confirm-cancel");
    assert!(!open_dialog(&r, cx));
    assert_eq!(deletes(&r), 0);
    assert!(still_there(&r, cx));
    assert!(timeline_focused(cx, &tl));
}

#[gpui::test]
async fn a_click_outside_keeps_the_message(cx: &mut TestAppContext) {
    let (r, cx) = start(cx);
    let tl = main_timeline(&r, cx);
    ask(cx, &tl);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.simulate_click(point(px(2.0), px(2.0)), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(!open_dialog(&r, cx));
    assert_eq!(deletes(&r), 0);
    assert!(still_there(&r, cx));
    assert!(timeline_focused(cx, &tl));
}

#[gpui::test]
async fn changing_conversation_closes_the_dialog(cx: &mut TestAppContext) {
    let (r, cx) = start(cx);
    let tl = main_timeline(&r, cx);
    ask(cx, &tl);
    assert!(open_dialog(&r, cx));
    tl.update(cx, |t, cx| t.set_conversation(cid("D2"), cx));
    cx.run_until_parked();
    assert!(!open_dialog(&r, cx));
    assert_eq!(deletes(&r), 0);
}

/// Opens a pop-out of `D1` and asks to delete in it.
fn popout_ask(
    r: &Rig,
    cx: &mut VisualTestContext,
) -> (
    gpui::WindowHandle<parla::popout::PopoutView>,
    Entity<Timeline<WorkspaceSession>>,
) {
    let ws = r.env.providers[0].workspace().clone();
    let handle = cx
        .update(|_, cx| parla::popout::open(PopOutTarget::conversation(ws, cid("D1")), cx))
        .expect("popout opens");
    settle(cx);
    let tl = cx.update(|_, cx| handle.update(cx, |v, _, _| v.timeline().unwrap()).unwrap());
    tl.update(cx, |t, cx| t.select(Some(mine()), cx));
    cx.update(|_, cx| {
        cx.update_window(handle.into(), |_, window, cx| {
            window.focus(&tl.read(cx).focus_handle(cx), cx);
            window.dispatch_action(Box::new(DeleteMessage), cx);
        })
        .unwrap();
    });
    settle(cx);
    (handle, tl)
}

fn popout_key(
    cx: &mut VisualTestContext,
    handle: &gpui::WindowHandle<parla::popout::PopoutView>,
    key: &str,
) {
    cx.update(|_, cx| {
        cx.update_window((*handle).into(), |_, window, cx| {
            window.dispatch_keystroke(
                Keystroke::parse_with_os(key, cx.platform_info().os).unwrap(),
                cx,
            );
        })
        .unwrap();
    });
    settle(cx);
}

fn popout_state(
    cx: &mut VisualTestContext,
    handle: &gpui::WindowHandle<parla::popout::PopoutView>,
    tl: &Entity<Timeline<WorkspaceSession>>,
) -> (bool, bool) {
    cx.update(|_, cx| {
        handle
            .update(cx, |v, window, cx| {
                (
                    v.delete_confirm().is_some(),
                    tl.read(cx).focus_handle(cx).is_focused(window),
                )
            })
            .unwrap()
    })
}

#[gpui::test]
async fn popout_confirms_and_escape_keeps(cx: &mut TestAppContext) {
    let (r, cx) = start(cx);
    let (handle, tl) = popout_ask(&r, cx);
    assert!(popout_state(cx, &handle, &tl).0, "the pop-out shows it");
    assert!(!open_dialog(&r, cx), "the main window does not");
    popout_key(cx, &handle, "escape");
    assert_eq!(popout_state(cx, &handle, &tl), (false, true));
    assert_eq!(deletes(&r), 0);
    assert!(still_there(&r, cx));

    tl.update(cx, |t, cx| t.select(Some(mine()), cx));
    cx.update(|_, cx| {
        cx.update_window(handle.into(), |_, window, cx| {
            window.dispatch_action(Box::new(DeleteMessage), cx);
        })
        .unwrap();
    });
    settle(cx);
    assert!(popout_state(cx, &handle, &tl).0);
    popout_key(cx, &handle, "enter");
    assert_eq!(popout_state(cx, &handle, &tl), (false, true));
    assert_eq!(deletes(&r), 1);
}
