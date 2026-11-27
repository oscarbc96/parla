//! Edits, replacements, thread summaries and deleted files reach every held
//! copy of a message: channel window, open thread replies and roots.

use std::sync::Arc;

use domain::test_support::{cid, conv, mid, msg, ts, uid};
use domain::{
    Changed, Effect, FileId, FileKind, FileRef, HistoryPage, Input, Message, PersistOp,
    ProviderEvent, ProviderExt, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn reply(t: &str) -> Message {
    let mut m = msg("c1", t, "reply");
    m.thread_root = Some(ts("1.000000"));
    m
}

fn file(id: &str) -> FileRef {
    FileRef {
        id: FileId(Arc::from(id)),
        name: Arc::from("a.png"),
        title: None,
        mime: Arc::from("image/png"),
        size: 1,
        kind: FileKind::Image,
        thumbs: Arc::from([]),
        download: None,
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

/// A channel window with the root, and its thread open with one reply.
fn state() -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.self_user = Some(uid("me"));
    s.conversations.insert(cid("c1"), conv("c1", "general"));
    let mut root = msg("c1", "1.000000", "root");
    root.files = Arc::from([file("F1")]);
    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: vec![root.clone()],
                has_more: false,
            },
        },
    );
    reduce(&mut s, Input::OpenThread(mid("c1", "1.000000")));
    let mut r = reply("2.000000");
    r.files = Arc::from([file("F1")]);
    reduce(
        &mut s,
        Input::RepliesLoaded {
            root: mid("c1", "1.000000"),
            page: HistoryPage {
                messages: vec![root, r],
                has_more: false,
            },
        },
    );
    s
}

fn thread(s: &WorkspaceState) -> &domain::ThreadWindow {
    &s.threads[&mid("c1", "1.000000")]
}

fn has(fx: &[Effect], c: &Changed) -> bool {
    fx.iter().any(|e| matches!(e, Effect::Changed(x) if x == c))
}

#[test]
fn edit_reaches_thread_reply_root_and_channel_copy() {
    let mut s = state();
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessageEdited {
            id: mid("c1", "2.000000"),
            text: Arc::from("new"),
            plain: Arc::from("new plain"),
            edited: ts("3.000000"),
        }),
    );
    let r = &thread(&s).replies.messages()[0];
    assert_eq!((&*r.text, &*r.plain), ("new", "new plain"));
    assert_eq!(r.edited, Some(ts("3.000000")));
    assert!(has(&fx, &Changed::Thread(mid("c1", "1.000000"))));
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Persist(PersistOp::EditMessage { plain, .. }) if &**plain == "new plain"
    )));
    // The root is held by the window and the thread.
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessageEdited {
            id: mid("c1", "1.000000"),
            text: Arc::from("root2"),
            plain: Arc::from("root2"),
            edited: ts("4.000000"),
        }),
    );
    assert_eq!(&*s.windows[&cid("c1")].messages()[0].text, "root2");
    assert_eq!(&*thread(&s).root.as_ref().unwrap().text, "root2");
}

#[test]
fn message_updated_replaces_every_copy_and_keeps_flags() {
    let mut s = state();
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::PinChanged {
            id: mid("c1", "1.000000"),
            pinned: true,
        }),
    );
    assert!(s.windows[&cid("c1")].messages()[0].pinned);
    let mut new = msg("c1", "1.000000", "root");
    new.files = Arc::from([]);
    let fx = reduce(&mut s, Input::Provider(ProviderEvent::MessageUpdated(new)));
    let w = &s.windows[&cid("c1")].messages()[0];
    assert!(w.files.is_empty() && w.pinned);
    assert!(thread(&s).root.as_ref().unwrap().files.is_empty());
    assert!(has(&fx, &Changed::Timeline(cid("c1"))));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::ReplaceMessage(_))))
    );
}

#[test]
fn thread_updated_sets_summary_on_held_roots() {
    let mut s = state();
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::ThreadUpdated {
            root: mid("c1", "1.000000"),
            reply_count: 5,
            latest_reply: Some(ts("9.000000")),
            reply_users: Arc::from([uid("u2")]),
        }),
    );
    let w = &s.windows[&cid("c1")].messages()[0];
    assert_eq!(w.reply_count, 5);
    assert_eq!(w.latest_reply, Some(ts("9.000000")));
    assert_eq!(thread(&s).root.as_ref().unwrap().reply_count, 5);
    assert!(has(&fx, &Changed::Timeline(cid("c1"))));
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Persist(PersistOp::ThreadSummary { reply_count: 5, .. })
    )));
}

#[test]
fn file_deleted_tombstones_every_copy_and_persists() {
    let mut s = state();
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::FileDeleted(FileId(Arc::from("F1")))),
    );
    assert!(s.windows[&cid("c1")].messages()[0].files[0].tombstone);
    assert!(thread(&s).replies.messages()[0].files[0].tombstone);
    assert!(thread(&s).root.as_ref().unwrap().files[0].tombstone);
    assert!(has(&fx, &Changed::Timeline(cid("c1"))));
    assert!(has(&fx, &Changed::Thread(mid("c1", "1.000000"))));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::FileDeleted(_))))
    );
}
