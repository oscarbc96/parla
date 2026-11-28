use std::cell::{Cell, RefCell};
use std::rc::Rc;

use components::{
    Cancel, Composer, ComposerEvent, ComposerSource, EditLast, Newline, Send, TimelineEvent,
    composer::Paste,
};
use domain::test_support::{cid, mid, msg, uid};
use domain::{
    Capabilities, ComposedText, ConversationId, DeliveryState, HistoryPage, Input, MessageWindow,
    UserId, WorkspaceState, reduce,
};
use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use settings::WorkspaceKey;

struct Src {
    state: WorkspaceState,
    me: UserId,
    caps: Capabilities,
    read_only: bool,
    uploads: Vec<Vec<std::path::PathBuf>>,
    must_join: bool,
    joins: Vec<ConversationId>,
}

impl ComposerSource for Src {
    fn stage(
        &mut self,
        _c: ConversationId,
        _thread: Option<domain::MessageTs>,
        paths: Vec<std::path::PathBuf>,
        _cx: &mut gpui::App,
    ) {
        self.uploads.push(paths);
    }
    fn read_only(&self) -> bool {
        self.read_only
    }
    fn must_join(&self, _c: &ConversationId) -> bool {
        self.must_join
    }
    fn join_conversation(&mut self, c: ConversationId, _cx: &mut gpui::App) {
        self.joins.push(c);
    }
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn self_user(&self) -> Option<&UserId> {
        Some(&self.me)
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
}

type Events = Rc<RefCell<Vec<ComposerEvent>>>;

fn composer_source(cx: &mut TestAppContext, caps: Capabilities) -> Entity<Src> {
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    let mut own_old = msg("C1", "1000.000001", "old own");
    own_old.author = Some(uid("u1"));
    let mut other = msg("C1", "1100.000001", "theirs");
    other.author = Some(uid("u2"));
    let mut own_new = msg("C1", "1200.000001", "newest own");
    own_new.author = Some(uid("u1"));
    let mut pending = msg("C1", "1300.000001", "pending own");
    pending.author = Some(uid("u1"));
    pending.state = DeliveryState::Pending;
    reduce(
        &mut state,
        Input::HistoryLoaded {
            conversation: cid("C1"),
            page: HistoryPage {
                messages: vec![own_old, other, own_new, pending],
                has_more: false,
            },
        },
    );
    let src = cx.new(|_| Src {
        state,
        me: uid("u1"),
        caps,
        read_only: false,
        uploads: Vec::new(),
        must_join: false,
        joins: Vec::new(),
    });
    SRC.with(|s| *s.borrow_mut() = Some(src.clone()));
    src
}

fn composer(cx: &mut TestAppContext, caps: Capabilities) -> Entity<Composer<Src>> {
    let src = composer_source(cx, caps);
    cx.new(|cx| Composer::new(src, cid("C1"), cx))
}

fn setup(
    cx: &mut TestAppContext,
    caps: Capabilities,
) -> (Entity<Composer<Src>>, Events, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let src = composer_source(cx, caps);
    let (view, vcx) = cx.add_window_view({
        let src = src.clone();
        move |_w, cx| Composer::new(src, cid("C1"), cx)
    });
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
    (view, events, vcx)
}

thread_local! {
    static SRC: RefCell<Option<Entity<Src>>> = const { RefCell::new(None) };
}

fn edit_caps() -> Capabilities {
    Capabilities {
        edit: true,
        ..Default::default()
    }
}

fn text(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) -> String {
    v.read_with(vcx, |c, cx| c.text(cx).to_string())
}

#[gpui::test]
fn enter_submits_and_clears(cx: &mut TestAppContext) {
    let (v, ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("  hello ");
    vcx.dispatch_action(Send);
    assert_eq!(
        *ev.borrow(),
        [ComposerEvent::Submit {
            body: ComposedText::plain("hello")
        }]
    );
    assert_eq!(text(&v, vcx), "");
}

#[gpui::test]
fn enter_key_is_bound_to_send(cx: &mut TestAppContext) {
    let (_v, ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_keystrokes("h i enter");
    assert_eq!(
        *ev.borrow(),
        [ComposerEvent::Submit {
            body: ComposedText::plain("hi")
        }]
    );
}

#[gpui::test]
fn delete_to_line_start_in_the_composer_never_deletes_the_message(cx: &mut TestAppContext) {
    // The platform's "delete to line start" key; the timeline binds the
    // same key to its message delete here, as the macOS keymap does.
    let key = if cfg!(target_os = "macos") {
        "cmd-backspace"
    } else {
        "ctrl-shift-backspace"
    };
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let comp = composer(cx, edit_caps());
    let deletes = Rc::new(Cell::new(0));
    struct InTimeline {
        composer: Entity<Composer<Src>>,
        deletes: Rc<Cell<usize>>,
    }
    impl gpui::Render for InTimeline {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::{InteractiveElement as _, ParentElement as _, Styled as _, div};
            let deletes = self.deletes.clone();
            div()
                .key_context("Timeline")
                .size_full()
                .on_action(move |_: &components::timeline::DeleteMessage, _, _| {
                    deletes.set(deletes.get() + 1);
                })
                .child(self.composer.clone())
        }
    }
    let host = InTimeline {
        composer: comp.clone(),
        deletes: deletes.clone(),
    };
    let (_host, vcx) = cx.add_window_view(move |_w, _| host);
    vcx.update(|w, cx| {
        let handle = comp.read(cx).focus_handle(cx);
        w.focus(&handle, cx);
        cx.bind_keys([gpui::KeyBinding::new(
            key,
            components::timeline::DeleteMessage,
            Some("Timeline"),
        )]);
    });
    comp.update(vcx, |c, cx| c.set_text("one\ntwo\nthree".into(), cx));
    vcx.run_until_parked();
    vcx.simulate_keystrokes(key);
    assert_eq!(deletes.get(), 0, "the message delete action must not fire");
    assert_eq!(text(&comp, vcx), "one\ntwo\n");
}

#[gpui::test]
fn shift_enter_inserts_newline(cx: &mut TestAppContext) {
    let (v, ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("a");
    vcx.dispatch_action(Newline);
    vcx.simulate_input("b");
    assert_eq!(text(&v, vcx), "a\nb");
    assert!(ev.borrow().is_empty());
}

#[gpui::test]
fn blank_never_submits(cx: &mut TestAppContext) {
    let (v, ev, vcx) = setup(cx, edit_caps());
    vcx.dispatch_action(Send);
    vcx.simulate_input("  ");
    vcx.dispatch_action(Newline);
    vcx.dispatch_action(Send);
    assert!(ev.borrow().is_empty());
    assert_eq!(text(&v, vcx), "  \n");
}

#[gpui::test]
fn up_in_empty_edits_last_own(cx: &mut TestAppContext) {
    let (v, ev, vcx) = setup(cx, edit_caps());
    vcx.dispatch_action(EditLast);
    assert_eq!(text(&v, vcx), "newest own");
    assert_eq!(
        v.read_with(vcx, |c, _| c.editing().cloned()),
        Some(mid("C1", "1200.000001"))
    );
    vcx.simulate_input("!");
    vcx.dispatch_action(Send);
    assert_eq!(
        *ev.borrow(),
        [ComposerEvent::SubmitEdit {
            id: mid("C1", "1200.000001"),
            body: ComposedText::plain("newest own!")
        }]
    );
    assert_eq!(text(&v, vcx), "");
    assert!(v.read_with(vcx, |c, _| c.editing().is_none()));
}

#[gpui::test]
fn up_does_nothing_when_not_empty_or_unsupported(cx: &mut TestAppContext) {
    let (v, _ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("x");
    vcx.dispatch_action(EditLast);
    assert_eq!(text(&v, vcx), "x");
    assert!(v.read_with(vcx, |c, _| c.editing().is_none()));

    let (v, _ev, vcx) = setup(cx, Capabilities::default());
    vcx.dispatch_action(EditLast);
    assert_eq!(text(&v, vcx), "");
}

#[gpui::test]
fn escape_cancels_edit(cx: &mut TestAppContext) {
    let (v, ev, vcx) = setup(cx, edit_caps());
    vcx.dispatch_action(EditLast);
    vcx.dispatch_action(Cancel);
    assert_eq!(*ev.borrow(), [ComposerEvent::Cancel]);
    assert_eq!(text(&v, vcx), "");
    assert!(v.read_with(vcx, |c, _| c.editing().is_none()));
}

#[gpui::test]
fn timeline_edit_request_enters_edit_mode(cx: &mut TestAppContext) {
    let (v, ev, vcx) = setup(cx, edit_caps());
    let id = mid("C1", "1000.000001");
    vcx.update(|_, cx| {
        v.update(cx, |c, cx| {
            c.handle_timeline_event(
                &TimelineEvent::EditRequested {
                    id: id.clone(),
                    text: "old own".into(),
                },
                cx,
            )
        })
    });
    assert_eq!(text(&v, vcx), "old own");
    vcx.simulate_input("!");
    vcx.dispatch_action(Send);
    assert_eq!(
        *ev.borrow(),
        [ComposerEvent::SubmitEdit {
            id,
            body: ComposedText::plain("old own!")
        }]
    );
}

#[gpui::test]
fn editing_keys_work(cx: &mut TestAppContext) {
    let (v, _ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("abc");
    vcx.simulate_keystrokes("backspace left x");
    assert_eq!(text(&v, vcx), "axb");
}

#[gpui::test]
fn enter_during_composition_does_not_send(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler as _;
    let (v, ev, vcx) = setup(cx, edit_caps());
    let input = v.read_with(vcx, |c, _| c.input().clone());
    vcx.update(|w, cx| {
        input.update(cx, |t, cx| {
            t.replace_and_mark_text_in_range(None, "にほ", None, w, cx)
        })
    });
    vcx.dispatch_action(Send);
    assert!(ev.borrow().is_empty());
    assert_eq!(text(&v, vcx), "にほ");
    assert!(!input.read_with(vcx, |t, _| t.is_composing()));
    // The next Enter sends the committed text.
    vcx.dispatch_action(Send);
    assert_eq!(
        *ev.borrow(),
        [ComposerEvent::Submit {
            body: ComposedText::plain("にほ")
        }]
    );
}

#[gpui::test]
fn selection_and_clipboard_work_in_the_composer(cx: &mut TestAppContext) {
    use components::text_input::{Copy, Cut, SelectAll};
    let (v, _ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("hello");
    vcx.dispatch_action(SelectAll);
    vcx.dispatch_action(Cut);
    assert_eq!(text(&v, vcx), "");
    vcx.dispatch_action(Paste);
    assert_eq!(text(&v, vcx), "hello");
    vcx.dispatch_action(SelectAll);
    vcx.dispatch_action(Copy);
    assert_eq!(text(&v, vcx), "hello");
}

#[gpui::test]
fn up_moves_the_caret_when_the_composer_has_text(cx: &mut TestAppContext) {
    let (v, _ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("ab");
    vcx.dispatch_action(Newline);
    vcx.simulate_input("cd");
    vcx.run_until_parked();
    vcx.dispatch_action(EditLast);
    let head = v.read_with(vcx, |c, cx| c.input().read(cx).selection().head);
    assert_eq!(head, 2);
    assert_eq!(text(&v, vcx), "ab\ncd");
}

#[gpui::test]
fn changing_conversation_closes_menus_and_notices(cx: &mut TestAppContext) {
    let (v, _ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("hello");
    v.update_in(vcx, |c, window, cx| {
        c.begin_link(window, cx);
        c.open_send_later(cx);
    });
    assert!(v.read_with(vcx, |c, _| c.link_prompt_open() || c.send_later_open()));
    v.update(vcx, |c, cx| c.set_conversation(cid("C2"), cx));
    v.read_with(vcx, |c, _| {
        assert!(!c.link_prompt_open());
        assert!(!c.send_later_open());
    });
    assert_eq!(text(&v, vcx), "");
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    vcx.run_until_parked();
    let b = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    vcx.simulate_click(b.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
}

#[gpui::test]
fn send_button_submits_clears_and_keeps_the_input_focused(cx: &mut TestAppContext) {
    let (v, ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("  hello ");
    click(vcx, "composer-send");
    assert_eq!(
        *ev.borrow(),
        [ComposerEvent::Submit {
            body: ComposedText::plain("hello")
        }]
    );
    assert_eq!(text(&v, vcx), "");
    vcx.run_until_parked();
    let focused = vcx.update(|w, cx| v.read(cx).focus_handle(cx).is_focused(w));
    assert!(focused, "typing after Send goes to the input");
}

#[gpui::test]
fn send_button_does_nothing_when_empty(cx: &mut TestAppContext) {
    let (_v, ev, vcx) = setup(cx, edit_caps());
    click(vcx, "composer-send");
    assert!(ev.borrow().is_empty());
    vcx.simulate_input("   ");
    click(vcx, "composer-send");
    assert!(ev.borrow().is_empty());
}

#[gpui::test]
fn expired_session_swaps_the_input_for_a_read_only_strip(cx: &mut TestAppContext) {
    let (v, _ev, vcx) = setup(cx, edit_caps());
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("composer-input").is_some());
    assert!(vcx.debug_bounds("composer-read-only").is_none());
    let src = SRC.with(|s| s.borrow().clone().unwrap());
    src.update(vcx, |s, cx| {
        s.read_only = true;
        cx.notify();
    });
    v.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("composer-read-only").is_some());
    assert!(vcx.debug_bounds("composer-input").is_none());
}

#[gpui::test]
fn a_stale_send_while_read_only_keeps_the_draft_and_sends_nothing(cx: &mut TestAppContext) {
    let (v, ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("unsent draft");
    let src = SRC.with(|s| s.borrow().clone().unwrap());
    src.update(vcx, |s, _| s.read_only = true);
    v.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    vcx.dispatch_action(Send);
    assert!(ev.borrow().is_empty());
    assert_eq!(text(&v, vcx), "unsent draft");
}

#[gpui::test]
fn attach_and_paste_while_read_only_stage_nothing_and_keep_the_draft(cx: &mut TestAppContext) {
    let (v, _ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("unsent draft");
    let src = SRC.with(|s| s.borrow().clone().unwrap());
    src.update(vcx, |s, _| s.read_only = true);
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, b"hi").unwrap();
    v.update(vcx, |c, cx| {
        c.attach_paths(vec![f.clone()], cx);
        let img = gpui::Image::from_bytes(gpui::ImageFormat::Png, vec![0u8; 8]);
        c.paste_image(&img, cx);
    });
    vcx.run_until_parked();
    assert!(src.read_with(vcx, |s, _| s.uploads.is_empty()));
    assert_eq!(text(&v, vcx), "unsent draft");
    // Signed in again, the same call stages.
    src.update(vcx, |s, _| s.read_only = false);
    v.update(vcx, |c, cx| c.attach_paths(vec![f.clone()], cx));
    assert_eq!(src.read_with(vcx, |s, _| s.uploads.len()), 1);
}

#[gpui::test]
fn code_font_follows_a_settings_reload(cx: &mut TestAppContext) {
    use gpui::BorrowAppContext as _;
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path().join("cfg"));
    std::fs::create_dir_all(&paths.root).unwrap();
    std::fs::write(&paths.settings, "{}").unwrap();
    let file = paths.settings.clone();
    cx.update(|cx| cx.set_global(settings::SettingsStore::load(paths)));
    let (view, _events, vcx) = setup(cx, Capabilities::default());
    assert_eq!(
        view.read_with(vcx, |c, _| c.code_font().to_string()),
        "JetBrains Mono"
    );
    std::fs::write(&file, r#"{"buffer_font_family": "Menlo"}"#).unwrap();
    vcx.update(|_, cx| {
        cx.update_global::<settings::SettingsStore, _>(|s, _| {
            s.reload();
        })
    });
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |c, _| c.code_font().to_string()),
        "Menlo"
    );
}

/// Installs a settings store whose file holds `json`; returns the file.
fn settings_with(cx: &mut TestAppContext, json: &str) -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path().join("cfg"));
    std::fs::create_dir_all(&paths.root).unwrap();
    std::fs::write(&paths.settings, json).unwrap();
    let file = paths.settings.clone();
    std::mem::forget(dir);
    cx.update(|cx| cx.set_global(settings::SettingsStore::load(paths)));
    file
}

#[gpui::test]
fn cmd_enter_mode_sends_on_cmd_enter_and_newlines_on_enter(cx: &mut TestAppContext) {
    settings_with(cx, r#"{"send_key": "cmd_enter"}"#);
    let (v, ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("a");
    vcx.simulate_keystrokes("enter");
    vcx.simulate_input("b");
    assert_eq!(text(&v, vcx), "a\nb");
    assert!(ev.borrow().is_empty());
    let send = if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    };
    vcx.simulate_keystrokes(send);
    assert_eq!(
        *ev.borrow(),
        [ComposerEvent::Submit {
            body: ComposedText::plain("a\nb")
        }]
    );
}

#[gpui::test]
fn default_mode_enter_sends_and_cmd_enter_does_not(cx: &mut TestAppContext) {
    settings_with(cx, "{}");
    let (_v, ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("a");
    let cmd = if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    };
    vcx.simulate_keystrokes(cmd);
    assert!(ev.borrow().is_empty());
    vcx.simulate_keystrokes("enter");
    assert_eq!(ev.borrow().len(), 1);
}

#[gpui::test]
fn send_key_follows_a_settings_reload(cx: &mut TestAppContext) {
    use gpui::BorrowAppContext as _;
    let file = settings_with(cx, "{}");
    let (v, ev, vcx) = setup(cx, edit_caps());
    std::fs::write(&file, r#"{"send_key": "cmd_enter"}"#).unwrap();
    vcx.update(|_, cx| cx.update_global::<settings::SettingsStore, _>(|s, _| s.reload()));
    vcx.run_until_parked();
    vcx.simulate_input("a");
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&v, vcx), "a\n");
    assert!(ev.borrow().is_empty());
}

#[gpui::test]
fn footer_hint_follows_editing_and_send_key(cx: &mut TestAppContext) {
    settings_with(cx, "{}");
    let (v, _ev, vcx) = setup(cx, edit_caps());
    let hint = |v: &gpui::Entity<_>, vcx: &mut gpui::VisualTestContext| {
        v.read_with(vcx, |c: &components::composer::Composer<_>, _| {
            c.footer_hint().to_string()
        })
    };
    assert_eq!(hint(&v, vcx), "\u{23ce} send · \u{21e7}\u{23ce} new line");
    vcx.dispatch_action(EditLast);
    assert_eq!(hint(&v, vcx), "esc to cancel · enter to save");
    vcx.dispatch_action(Cancel);
    assert_eq!(hint(&v, vcx), "\u{23ce} send · \u{21e7}\u{23ce} new line");
}

#[gpui::test]
fn a_channel_the_user_is_not_in_offers_join_and_sends_nothing(cx: &mut TestAppContext) {
    let (v, ev, vcx) = setup(cx, edit_caps());
    vcx.simulate_input("first words");
    let src = SRC.with(|s| s.borrow().clone().unwrap());
    src.update(vcx, |s, _| s.must_join = true);
    v.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("composer-join").is_some());
    assert!(vcx.debug_bounds("composer-input").is_none());
    // Sending first needs the join; the draft stays.
    vcx.dispatch_action(Send);
    assert!(ev.borrow().is_empty());
    assert_eq!(text(&v, vcx), "first words");
    click(vcx, "composer-join-button");
    src.read_with(vcx, |s, _| assert_eq!(s.joins, [cid("C1")]));
    // Joined: the input is back with the draft.
    src.update(vcx, |s, _| s.must_join = false);
    v.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("composer-input").is_some());
    assert_eq!(text(&v, vcx), "first words");
}
