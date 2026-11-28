//! Files staged on the composer draft: chips, remove, cancel, retry, and Send with
//! the files and the text together.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use components::{Composer, ComposerSource, StagedFile, StagedState};
use domain::test_support::{cid, uid};
use domain::{Capabilities, ConversationId, MessageTs, MessageWindow, UserId, WorkspaceState};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use settings::WorkspaceKey;

type Log = Rc<RefCell<Vec<String>>>;

/// A session stand-in that keeps the staged files in memory and logs every call.
struct Src {
    state: WorkspaceState,
    me: UserId,
    caps: Capabilities,
    staged: Vec<StagedFile>,
    next: u64,
    log: Log,
    online: bool,
    restored: Option<String>,
    snippets: bool,
}

impl ComposerSource for Src {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn self_user(&self) -> Option<&UserId> {
        Some(&self.me)
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn can_upload(&self) -> bool {
        self.online
    }
    fn take_restored_text(&mut self, _: &ConversationId, _: Option<&MessageTs>) -> Option<String> {
        self.restored.take()
    }
    fn staged(&self, _: &ConversationId, thread: Option<&MessageTs>) -> Vec<StagedFile> {
        // One draft per thread: files staged in a thread are not in the channel draft.
        if thread.is_some() {
            return self
                .staged
                .iter()
                .filter(|f| f.id >= 1000)
                .cloned()
                .collect();
        }
        self.staged
            .iter()
            .filter(|f| f.id < 1000)
            .cloned()
            .collect()
    }
    fn stage(
        &mut self,
        _: ConversationId,
        thread: Option<MessageTs>,
        paths: Vec<PathBuf>,
        _: &mut gpui::App,
    ) {
        for p in paths {
            self.next += 1;
            let id = self.next + if thread.is_some() { 1000 } else { 0 };
            self.log.borrow_mut().push(format!(
                "stage {} thread={}",
                p.file_name().unwrap().to_string_lossy(),
                thread.is_some()
            ));
            self.staged.push(StagedFile {
                id,
                name: p.file_name().unwrap().to_string_lossy().to_string().into(),
                size: 2,
                thumb: None,
                snippet: None,
                state: StagedState::Ready,
            });
        }
    }
    fn unstage(&mut self, _: ConversationId, _: Option<MessageTs>, id: u64, _: &mut gpui::App) {
        self.log.borrow_mut().push(format!("unstage {id}"));
        self.staged.retain(|f| f.id != id);
    }
    fn cancel_staged(
        &mut self,
        _: ConversationId,
        _: Option<MessageTs>,
        id: u64,
        _: &mut gpui::App,
    ) {
        self.log.borrow_mut().push(format!("cancel {id}"));
        for f in &mut self.staged {
            f.state = StagedState::Cancelled;
        }
    }
    fn retry_staged(
        &mut self,
        _: ConversationId,
        _: Option<MessageTs>,
        id: u64,
        _: &mut gpui::App,
    ) {
        self.log.borrow_mut().push(format!("retry {id}"));
        for f in &mut self.staged {
            f.state = StagedState::Uploading(None);
        }
    }
    fn supports_snippets(&self) -> bool {
        self.snippets
    }
    fn stage_snippet(
        &mut self,
        _: ConversationId,
        _: Option<MessageTs>,
        s: components::NewSnippet,
        _: &mut gpui::App,
    ) {
        self.log.borrow_mut().push(if s.as_file {
            format!("file name={} lines={}", s.name, s.text.lines().count())
        } else {
            format!(
                "snippet name={} lang={} lines={}",
                s.name,
                s.language,
                s.text.lines().count()
            )
        });
        self.next += 1;
        self.staged.push(StagedFile {
            id: self.next,
            name: s.name.into(),
            size: s.text.len() as u64,
            thumb: None,
            snippet: Some(components::StagedSnippet {
                language: s.language.into(),
                lines: s.text.lines().count(),
            }),
            state: StagedState::Ready,
        });
    }
    fn send_staged(
        &mut self,
        _: ConversationId,
        thread: Option<MessageTs>,
        text: Option<String>,
        _: &mut gpui::App,
    ) {
        self.log
            .borrow_mut()
            .push(format!("send {text:?} thread={}", thread.is_some()));
        for f in &mut self.staged {
            f.state = StagedState::Uploading(None);
        }
    }
}

fn setup(
    cx: &mut TestAppContext,
    reply: bool,
) -> (
    Entity<Composer<Src>>,
    Entity<Src>,
    Log,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let log: Log = Rc::default();
    let l = log.clone();
    let src = cx.new(move |_| Src {
        state: WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 100),
        me: uid("u1"),
        caps: Capabilities::default(),
        staged: Vec::new(),
        next: 0,
        log: l,
        online: true,
        restored: None,
        snippets: true,
    });
    let s = src.clone();
    let (view, vcx) = cx.add_window_view(move |_w, cx| {
        let mut c = Composer::new(s, cid("C1"), cx);
        if reply {
            c.set_reply_root(Some(domain::test_support::mid("C1", "1.0")), cx);
        }
        c
    });
    vcx.update(|w, cx| {
        let handle = gpui::Focusable::focus_handle(view.read(cx), cx);
        w.focus(&handle, cx);
    });
    (view, src, log, vcx)
}

/// What the real session does after a change: notify, so the composer repaints.
fn settle(src: &Entity<Src>, vcx: &mut VisualTestContext) {
    src.update(vcx, |_, cx| cx.notify());
    vcx.update(|w, cx| w.draw(cx).clear(cx));
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    let at = vcx.debug_bounds(selector).expect(selector).center();
    vcx.simulate_click(at, gpui::Modifiers::none());
}

fn long_rust(lines: usize) -> String {
    let mut t = String::from("pub fn run() {\n");
    for i in 0..lines {
        t.push_str(&format!("    let v{i} = {i};\n"));
    }
    t.push_str("}\n");
    t
}

fn paste(vcx: &mut VisualTestContext, text: &str) {
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_string()));
    vcx.dispatch_action(components::composer::Paste);
}

#[gpui::test]
fn a_short_paste_goes_into_the_draft(cx: &mut TestAppContext) {
    let (comp, _src, _log, vcx) = setup(cx, false);
    paste(vcx, "one\ntwo");
    comp.read_with(vcx, |c, cx| {
        assert_eq!(c.text(cx), "one\ntwo");
        assert!(c.snippet_offer_lines().is_none());
    });
}

#[gpui::test]
fn a_long_paste_offers_a_snippet_instead_of_filling_the_draft(cx: &mut TestAppContext) {
    let (comp, src, _log, vcx) = setup(cx, false);
    paste(vcx, &long_rust(30));
    settle(&src, vcx);
    comp.read_with(vcx, |c, cx| {
        assert_eq!(c.text(cx), "", "the draft is untouched");
        assert_eq!(c.snippet_offer_lines(), Some(32));
        assert_eq!(c.snippet_offer_language().map(|l| l.id), Some("rust"));
    });
    assert!(vcx.debug_bounds("composer-snippet-offer").is_some());
}

/// The TextInput context binds the paste key too; the composer's own paste
/// (images, files, the snippet offer) must still run.
#[gpui::test]
fn the_composer_paste_key_still_offers_a_snippet(cx: &mut TestAppContext) {
    let (comp, src, _log, vcx) = setup(cx, false);
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string(long_rust(30)));
    let key = if cfg!(target_os = "macos") {
        "cmd-v"
    } else {
        "ctrl-v"
    };
    vcx.simulate_keystrokes(key);
    settle(&src, vcx);
    comp.read_with(vcx, |c, cx| {
        assert_eq!(c.text(cx), "", "the draft is untouched");
        assert_eq!(c.snippet_offer_lines(), Some(32));
    });
}

#[gpui::test]
fn the_threshold_is_lines_or_characters(cx: &mut TestAppContext) {
    let (comp, _src, _log, vcx) = setup(cx, false);
    paste(vcx, &"x\n".repeat(components::snippet::OFFER_MIN_LINES));
    comp.read_with(vcx, |c, _| {
        assert!(c.snippet_offer_lines().is_none(), "exactly the limit")
    });
    comp.update(vcx, |c, cx| c.set_text(String::new(), cx));
    paste(vcx, &"x".repeat(components::snippet::OFFER_MIN_CHARS + 1));
    comp.read_with(vcx, |c, _| assert_eq!(c.snippet_offer_lines(), Some(1)));
}

#[gpui::test]
fn without_snippet_support_a_long_paste_is_plain_text(cx: &mut TestAppContext) {
    let (comp, src, _log, vcx) = setup(cx, false);
    src.update(vcx, |s, _| s.snippets = false);
    let text = long_rust(30);
    paste(vcx, &text);
    comp.read_with(vcx, |c, cx| {
        assert_eq!(c.text(cx), text);
        assert!(c.snippet_offer_lines().is_none());
    });
}

#[gpui::test]
fn share_as_snippet_stages_it_with_title_and_language(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    paste(vcx, &long_rust(30));
    settle(&src, vcx);
    comp.update(vcx, |c, cx| c.set_snippet_title("contrast_check", cx));
    click(vcx, "composer-snippet-share");
    settle(&src, vcx);
    assert_eq!(
        *log.borrow(),
        ["snippet name=contrast_check.rs lang=rust lines=32"]
    );
    comp.read_with(vcx, |c, cx| {
        assert!(c.snippet_offer_lines().is_none(), "offer closed");
        assert_eq!(c.text(cx), "", "the text is in the chip, not the draft");
    });
    assert!(
        vcx.debug_bounds("composer-staged-1").is_some(),
        "snippet chip"
    );
    assert!(
        !log.borrow().iter().any(|l| l.starts_with("send")),
        "nothing is sent until Send"
    );
}

#[gpui::test]
fn the_language_can_be_changed_before_sharing(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    paste(vcx, &long_rust(30));
    comp.update(vcx, |c, cx| c.set_snippet_language("python", cx));
    settle(&src, vcx);
    click(vcx, "composer-snippet-share");
    // An empty title falls back to `snippet.<ext>`.
    assert_eq!(
        *log.borrow(),
        ["snippet name=snippet.py lang=python lines=32"]
    );
}

#[gpui::test]
fn paste_as_text_puts_the_text_in_the_draft(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    let text = long_rust(30);
    paste(vcx, &text);
    settle(&src, vcx);
    click(vcx, "composer-snippet-paste");
    comp.read_with(vcx, |c, cx| {
        assert_eq!(c.text(cx), text);
        assert!(c.snippet_offer_lines().is_none());
    });
    assert!(log.borrow().is_empty());
}

#[gpui::test]
fn discarding_the_offer_drops_the_paste(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    paste(vcx, &long_rust(30));
    settle(&src, vcx);
    click(vcx, "composer-snippet-dismiss");
    comp.read_with(vcx, |c, cx| {
        assert_eq!(c.text(cx), "");
        assert!(c.snippet_offer_lines().is_none());
    });
    assert!(log.borrow().is_empty());
}

fn focus_main(comp: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) {
    vcx.update(|w, cx| {
        let h = comp.read(cx).input().read(cx).focus_handle_for_test();
        w.focus(&h, cx);
    });
}

#[gpui::test]
fn switching_conversation_drops_the_offer(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    paste(vcx, &long_rust(30));
    settle(&src, vcx);
    comp.update(vcx, |c, cx| c.set_conversation(cid("C2"), cx));
    comp.update(vcx, |c, cx| c.create_snippet(cx));
    comp.read_with(vcx, |c, _| assert!(c.snippet_offer_lines().is_none()));
    assert!(
        log.borrow().is_empty(),
        "nothing staged into the new conversation"
    );
}

#[gpui::test]
fn enter_in_the_main_box_sends_and_keeps_the_offer(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    paste(vcx, &long_rust(30));
    settle(&src, vcx);
    comp.update(vcx, |c, cx| c.set_text("hello".into(), cx));
    focus_main(&comp, vcx);
    vcx.dispatch_action(components::Send);
    comp.read_with(vcx, |c, cx| {
        assert_eq!(c.text(cx), "", "the message went out");
        assert!(c.snippet_offer_lines().is_some(), "the offer is still open");
    });
    assert!(log.borrow().is_empty(), "no snippet was staged");
}

#[gpui::test]
fn escape_with_the_language_menu_open_closes_only_the_menu(cx: &mut TestAppContext) {
    let (comp, src, _log, vcx) = setup(cx, false);
    paste(vcx, &long_rust(30));
    settle(&src, vcx);
    click(vcx, "composer-snippet-language");
    assert!(vcx.debug_bounds("composer-snippet-languages").is_some());
    vcx.dispatch_action(components::Cancel);
    settle(&src, vcx);
    assert!(
        vcx.debug_bounds("composer-snippet-languages").is_none(),
        "menu closed"
    );
    comp.read_with(vcx, |c, cx| {
        assert!(c.snippet_offer_lines().is_some(), "offer kept");
        assert_eq!(c.text(cx), "");
    });
}

#[gpui::test]
fn a_second_long_paste_keeps_the_first_as_draft_text(cx: &mut TestAppContext) {
    let (comp, _src, _log, vcx) = setup(cx, false);
    let first = long_rust(30);
    paste(vcx, &first);
    paste(vcx, &long_rust(25));
    comp.read_with(vcx, |c, cx| {
        assert_eq!(c.text(cx), first, "the first paste is in the draft");
        assert_eq!(
            c.snippet_offer_lines(),
            Some(27),
            "the offer is for the second"
        );
    });
}

#[gpui::test]
fn a_paste_over_the_size_cap_is_refused_with_a_notice(cx: &mut TestAppContext) {
    let (comp, _src, _log, vcx) = setup(cx, false);
    paste(vcx, &"x\n".repeat(2 * 1024 * 1024));
    comp.read_with(vcx, |c, cx| {
        assert!(c.snippet_offer_lines().is_none());
        assert_eq!(c.text(cx), "");
        assert!(c.notice(&common::en()).is_some(), "the user is told");
    });
}

#[gpui::test]
fn an_oversized_paste_can_be_sent_as_a_file(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    let text = "x\n".repeat(components::snippet::MAX_SNIPPET_BYTES);
    paste(vcx, &text);
    settle(&src, vcx);
    click(vcx, "composer-paste-as-file");
    settle(&src, vcx);
    assert_eq!(
        *log.borrow(),
        [format!(
            "file name=paste.txt lines={}",
            components::snippet::MAX_SNIPPET_BYTES
        )]
    );
    comp.read_with(vcx, |c, cx| {
        assert!(
            c.notice(&common::en()).is_none(),
            "notice gone once acted on"
        );
        assert_eq!(c.text(cx), "");
    });
}

#[gpui::test]
fn the_language_menu_filters_as_you_type_and_follows_the_arrows(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    paste(vcx, &long_rust(30));
    settle(&src, vcx);
    click(vcx, "composer-snippet-language");
    settle(&src, vcx);
    assert!(vcx.debug_bounds("composer-snippet-filter").is_some());
    vcx.simulate_input("ru");
    settle(&src, vcx);
    assert!(vcx.debug_bounds("composer-snippet-lang-rust").is_some());
    assert!(vcx.debug_bounds("composer-snippet-lang-ruby").is_some());
    assert!(
        vcx.debug_bounds("composer-snippet-lang-python").is_none(),
        "filtered out"
    );
    // Rust is first and selected; Down moves to Ruby, Enter picks it and closes the menu.
    vcx.dispatch_action(components::text_input::Down);
    vcx.dispatch_action(components::Send);
    settle(&src, vcx);
    assert!(vcx.debug_bounds("composer-snippet-languages").is_none());
    comp.read_with(vcx, |c, _| {
        assert_eq!(c.snippet_offer_language().map(|l| l.id), Some("ruby"));
        assert!(
            c.snippet_offer_lines().is_some(),
            "Enter in the menu does not share"
        );
    });
    assert!(log.borrow().is_empty());
}

#[gpui::test]
fn enter_in_the_title_shares_the_snippet(cx: &mut TestAppContext) {
    let (_comp, src, log, vcx) = setup(cx, false);
    paste(vcx, &long_rust(30));
    settle(&src, vcx);
    vcx.dispatch_action(components::Send);
    assert_eq!(
        *log.borrow(),
        ["snippet name=snippet.rs lang=rust lines=32"]
    );
}

#[gpui::test]
fn the_staged_chip_names_the_language_in_the_ui_language(cx: &mut TestAppContext) {
    let (_comp, src, _log, vcx) = setup(cx, false);
    paste(vcx, "just prose\n".repeat(30).as_str());
    settle(&src, vcx);
    click(vcx, "composer-snippet-share");
    settle(&src, vcx);
    let chip = src.read_with(vcx, |s, _| s.staged[0].snippet.clone().unwrap());
    assert_eq!(&*chip.language, "text", "the chip carries the id");
    let en = components::snippet::language_by_id(&chip.language)
        .unwrap()
        .display(&common::en());
    assert_eq!(en, "Plain text");
}
