//! Files staged on the composer draft: chips, remove, cancel, retry, and Send with
//! the files and the text together.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use components::{Composer, ComposerSource, Send, StagedFile, StagedState};
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

fn files(dir: &std::path::Path, names: &[&str]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|n| {
            let p = dir.join(n);
            std::fs::write(&p, b"hi").unwrap();
            p
        })
        .collect()
}

#[gpui::test]
fn attaching_stages_files_without_uploading(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    let dir = tempfile::tempdir().unwrap();
    let paths = files(dir.path(), &["a.txt", "b.pdf"]);
    comp.update(vcx, |c, cx| c.attach_paths(paths, cx));
    settle(&src, vcx);
    assert_eq!(
        *log.borrow(),
        ["stage a.txt thread=false", "stage b.pdf thread=false"]
    );
    assert!(
        vcx.debug_bounds("composer-staged-1").is_some(),
        "chip for a.txt"
    );
    assert!(
        vcx.debug_bounds("composer-staged-2").is_some(),
        "chip for b.pdf"
    );
    assert!(
        !log.borrow().iter().any(|l| l.starts_with("send")),
        "nothing is sent until Send"
    );
}

#[gpui::test]
fn pasted_image_is_staged(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    let dir = tempfile::tempdir().unwrap();
    comp.update(vcx, |c, _| c.set_upload_dir(dir.path().join("up")));
    let mut png = Vec::new();
    image::DynamicImage::new_rgba8(2, 2)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let img = gpui::Image::from_bytes(gpui::ImageFormat::Png, png);
    comp.update(vcx, |c, cx| c.paste_image(&img, cx));
    settle(&src, vcx);
    assert_eq!(log.borrow().len(), 1);
    assert!(log.borrow()[0].starts_with("stage ") && log.borrow()[0].contains(".png"));
}

#[gpui::test]
fn remove_detaches_one_chip(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    let dir = tempfile::tempdir().unwrap();
    let paths = files(dir.path(), &["a.txt", "b.txt"]);
    comp.update(vcx, |c, cx| c.attach_paths(paths, cx));
    settle(&src, vcx);
    click(vcx, "composer-staged-remove-1");
    settle(&src, vcx);
    assert!(log.borrow().contains(&"unstage 1".to_string()));
    assert!(vcx.debug_bounds("composer-staged-1").is_none());
    assert!(vcx.debug_bounds("composer-staged-2").is_some());
}

#[gpui::test]
fn send_with_files_and_text_sends_them_together(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    let dir = tempfile::tempdir().unwrap();
    let paths = files(dir.path(), &["a.txt"]);
    comp.update(vcx, |c, cx| c.attach_paths(paths, cx));
    settle(&src, vcx);
    comp.update(vcx, |c, cx| c.set_text("look at this".into(), cx));
    vcx.dispatch_action(Send);
    assert_eq!(
        log.borrow().last().unwrap(),
        "send Some(\"look at this\") thread=false"
    );
    comp.read_with(vcx, |c, cx| {
        assert_eq!(c.text(cx), "", "the field is cleared")
    });
}

#[gpui::test]
fn empty_text_with_files_still_sends(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    let dir = tempfile::tempdir().unwrap();
    let paths = files(dir.path(), &["a.txt"]);
    comp.update(vcx, |c, cx| c.attach_paths(paths, cx));
    settle(&src, vcx);
    vcx.dispatch_action(Send);
    assert_eq!(log.borrow().last().unwrap(), "send None thread=false");
}

#[gpui::test]
fn send_is_disabled_without_text_or_files_and_enabled_with_files(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    vcx.dispatch_action(Send);
    assert!(log.borrow().is_empty(), "nothing to send");
    let dir = tempfile::tempdir().unwrap();
    comp.update(vcx, |c, cx| {
        c.attach_paths(files(dir.path(), &["a.txt"]), cx)
    });
    settle(&src, vcx);
    vcx.update(|_, cx| {
        assert_eq!(comp.read(cx).staged_files(cx).len(), 1);
    });
    vcx.dispatch_action(Send);
    assert_eq!(log.borrow().len(), 2, "stage then send");
}

#[gpui::test]
fn uploading_chip_cancels_and_failed_chip_retries(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    let dir = tempfile::tempdir().unwrap();
    comp.update(vcx, |c, cx| {
        c.attach_paths(files(dir.path(), &["a.txt"]), cx)
    });
    settle(&src, vcx);
    vcx.dispatch_action(Send);
    settle(&src, vcx);
    // Uploading: the X cancels, and a second Send does not send it again.
    assert!(vcx.debug_bounds("composer-staged-retry-1").is_none());
    vcx.dispatch_action(Send);
    assert_eq!(
        log.borrow()
            .iter()
            .filter(|l| l.starts_with("send"))
            .count(),
        1
    );
    click(vcx, "composer-staged-remove-1");
    settle(&src, vcx);
    assert!(log.borrow().contains(&"cancel 1".to_string()));
    // Cancelled: Retry is offered.
    click(vcx, "composer-staged-retry-1");
    settle(&src, vcx);
    assert!(log.borrow().contains(&"retry 1".to_string()));
}

#[gpui::test]
fn failed_chip_shows_retry_and_remove(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    let dir = tempfile::tempdir().unwrap();
    comp.update(vcx, |c, cx| {
        c.attach_paths(files(dir.path(), &["a.txt"]), cx)
    });
    src.update(vcx, |s, _| {
        s.staged[0].state = StagedState::Failed("network error".into())
    });
    settle(&src, vcx);
    assert!(vcx.debug_bounds("composer-staged-retry-1").is_some());
    // A failed file is not sent by Send again; only Retry does that.
    vcx.dispatch_action(Send);
    assert!(!log.borrow().iter().any(|l| l.starts_with("send")));
    click(vcx, "composer-staged-remove-1");
    assert!(log.borrow().contains(&"unstage 1".to_string()));
}

#[gpui::test]
fn thread_composer_stages_and_sends_in_the_thread(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, true);
    let dir = tempfile::tempdir().unwrap();
    comp.update(vcx, |c, cx| {
        c.attach_paths(files(dir.path(), &["t.txt"]), cx)
    });
    settle(&src, vcx);
    assert_eq!(log.borrow()[0], "stage t.txt thread=true");
    comp.update(vcx, |c, cx| c.set_text("in thread".into(), cx));
    vcx.dispatch_action(Send);
    assert_eq!(
        log.borrow().last().unwrap(),
        "send Some(\"in thread\") thread=true"
    );
}

#[gpui::test]
fn oversized_files_are_refused_not_staged(cx: &mut TestAppContext) {
    let (comp, _src, log, vcx) = setup(cx, false);
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(components::upload::MAX_UPLOAD_BYTES + 1)
        .unwrap();
    comp.update(vcx, |c, cx| c.attach_paths(vec![big], cx));
    assert!(log.borrow().is_empty());
    comp.read_with(vcx, |c, _| {
        assert!(c.notice(&common::en()).unwrap().contains("big.bin"))
    });
}

#[gpui::test]
fn send_that_cannot_upload_keeps_the_typed_text(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    let dir = tempfile::tempdir().unwrap();
    comp.update(vcx, |c, cx| {
        c.attach_paths(files(dir.path(), &["a.txt"]), cx)
    });
    settle(&src, vcx);
    src.update(vcx, |s, _| s.online = false);
    comp.update(vcx, |c, cx| c.set_text("do not lose me".into(), cx));
    vcx.dispatch_action(Send);
    comp.read_with(vcx, |c, cx| assert_eq!(c.text(cx), "do not lose me"));
    assert!(
        log.borrow().iter().any(|l| l.starts_with("send")),
        "the session is told, so it can show why"
    );
}

#[gpui::test]
fn restored_text_goes_back_into_an_empty_draft(cx: &mut TestAppContext) {
    let (comp, src, _log, vcx) = setup(cx, false);
    src.update(vcx, |s, _| s.restored = Some("my comment".into()));
    settle(&src, vcx);
    comp.read_with(vcx, |c, cx| assert_eq!(c.text(cx), "my comment"));
    // Handed back once.
    comp.update(vcx, |c, cx| c.set_text(String::new(), cx));
    settle(&src, vcx);
    comp.read_with(vcx, |c, cx| assert_eq!(c.text(cx), ""));
}

#[gpui::test]
fn restored_text_is_put_before_what_was_typed_since(cx: &mut TestAppContext) {
    let (comp, src, _log, vcx) = setup(cx, false);
    comp.update(vcx, |c, cx| c.set_text("typed since".into(), cx));
    src.update(vcx, |s, _| s.restored = Some("my comment".into()));
    settle(&src, vcx);
    comp.read_with(vcx, |c, cx| {
        assert_eq!(c.text(cx), "my comment\ntyped since")
    });
}

#[gpui::test]
fn sending_files_in_a_reply_resets_the_broadcast_toggle(cx: &mut TestAppContext) {
    use components::{ComposerEvent, composer::ToggleBroadcast};
    let (comp, src, _log, vcx) = setup(cx, true);
    let events: Rc<RefCell<Vec<ComposerEvent>>> = Rc::default();
    let sink = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&comp, move |_, e: &ComposerEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach()
    });
    vcx.dispatch_action(ToggleBroadcast);
    let dir = tempfile::tempdir().unwrap();
    comp.update(vcx, |c, cx| {
        c.attach_paths(files(dir.path(), &["a.txt"]), cx)
    });
    settle(&src, vcx);
    vcx.dispatch_action(Send);
    // Chips gone (sent), then a plain reply: the toggle is off again.
    src.update(vcx, |s, _| s.staged.clear());
    settle(&src, vcx);
    comp.update(vcx, |c, cx| c.set_text("later".into(), cx));
    vcx.dispatch_action(Send);
    let evs = events.borrow();
    match &evs[..] {
        [ComposerEvent::SubmitReply { broadcast, .. }] => assert!(!broadcast),
        other => panic!("{other:?}"),
    }
}

#[gpui::test]
fn a_slash_command_with_files_staged_runs_as_a_command(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    let dir = tempfile::tempdir().unwrap();
    comp.update(vcx, |c, cx| {
        c.attach_paths(files(dir.path(), &["a.txt"]), cx)
    });
    settle(&src, vcx);
    comp.update(vcx, |c, cx| c.set_text("/nosuchcommand".into(), cx));
    vcx.dispatch_action(Send);
    assert!(
        !log.borrow().iter().any(|l| l.starts_with("send")),
        "a command is never posted as the comment of the files"
    );
    assert_eq!(
        src.read_with(vcx, |s, _| s.staged.len()),
        1,
        "the file stays staged"
    );
}

#[gpui::test]
fn send_waits_while_a_file_uploads(cx: &mut TestAppContext) {
    let (comp, src, log, vcx) = setup(cx, false);
    let dir = tempfile::tempdir().unwrap();
    comp.update(vcx, |c, cx| {
        c.attach_paths(files(dir.path(), &["a.txt"]), cx)
    });
    settle(&src, vcx);
    vcx.dispatch_action(Send);
    settle(&src, vcx);
    comp.update(vcx, |c, cx| c.set_text("next".into(), cx));
    vcx.dispatch_action(Send);
    assert_eq!(
        log.borrow()
            .iter()
            .filter(|l| l.starts_with("send"))
            .count(),
        1
    );
    comp.read_with(vcx, |c, cx| {
        assert_eq!(c.text(cx), "next", "the text is kept")
    });
    assert!(vcx.debug_bounds("composer-staged-hint").is_some());
}
