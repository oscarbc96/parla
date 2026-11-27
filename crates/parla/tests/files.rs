//! File download and upload through `WorkspaceSession` against `FakeProvider`.

mod support;

use std::sync::Arc;

use components::{FileNotice, StagedState};
use domain::test_support::{Call, FakeProvider, Op, cid, conv, member, uid};
use domain::{FileId, FileKind, FileRef, ProviderError, ProviderEvent, ProviderExt, RemoteUrl};
use gpui::{Entity, TestAppContext};
use parla::session::WorkspaceSession;

fn setup(cx: &mut TestAppContext) -> (Arc<FakeProvider>, Entity<WorkspaceSession>) {
    cx.update(|cx| {
        i18n::gpui::init(
            cx,
            &[components::catalog(), parla::catalog()],
            i18n::LocalePref::Code("en".parse().unwrap()),
        );
    });
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u1", "Me"));
    let session = support::memory_session(cx, provider.clone());
    (provider, session)
}

fn file(url: &RemoteUrl) -> FileRef {
    FileRef {
        id: FileId(Arc::from("F1")),
        name: Arc::from("report.pdf"),
        title: None,
        mime: Arc::from("application/pdf"),
        size: 3,
        kind: FileKind::Pdf,
        thumbs: Arc::from([]),
        download: Some(url.clone()),
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

#[gpui::test]
async fn download_unique_name_never_opens(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    let url = RemoteUrl(Arc::from("https://files.example/F1/report.pdf"));
    provider.add_file(url.clone(), b"pdf".to_vec());
    let dir = tempfile::tempdir().unwrap();
    session.update(cx, |s, _| s.set_download_dir(dir.path().to_path_buf()));
    let f = file(&url);
    session.update(cx, |s, cx| s.download(&f, cx));
    cx.run_until_parked();
    session.update(cx, |s, cx| s.download(&f, cx));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(dir.path().join("report.pdf")).unwrap(),
        b"pdf"
    );
    assert_eq!(
        std::fs::read(dir.path().join("report (1).pdf")).unwrap(),
        b"pdf"
    );
    let downloads = provider
        .calls()
        .iter()
        .filter(|c| matches!(c, Call::Download { .. }))
        .count();
    assert_eq!(downloads, 2);
    // Never opened: the session only reports where the file went.
    session.read_with(cx, |s, _| {
        assert_eq!(
            s.file_notice(),
            Some(FileNotice::Downloaded(dir.path().join("report (1).pdf")))
        );
    });
}

fn text_file(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, b"hi").unwrap();
    p
}

fn go_online(provider: &Arc<FakeProvider>, cx: &mut TestAppContext) {
    provider.push_event(ProviderEvent::Connected { reconnect: false });
    cx.run_until_parked();
}

fn uploads(provider: &FakeProvider) -> Vec<(usize, Option<String>)> {
    provider
        .calls()
        .iter()
        .filter_map(|c| match c {
            Call::Upload { files, text, .. } => Some((files.len(), text.clone())),
            _ => None,
        })
        .collect()
}

#[gpui::test]
async fn offline_send_errors_and_keeps_files_staged(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let a = text_file(dir.path(), "a.txt");
    session.update(cx, |s, cx| s.stage_files(cid("C1"), None, vec![a], cx));
    session.update(cx, |s, cx| {
        s.send_staged(cid("C1"), None, Some("see".into()), cx)
    });
    cx.run_until_parked();
    session.read_with(cx, |s, _| {
        match s.file_notice() {
            Some(FileNotice::Error(e)) => {
                assert_eq!((&*e.ns, &*e.id), ("parla", "file-error-needs-connection"))
            }
            other => panic!("{other:?}"),
        }
        assert!(s.state().outbox().is_empty());
        let staged = s.staged_files(&cid("C1"), None);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].state, StagedState::Ready);
    });
    assert!(uploads(&provider).is_empty());

    go_online(&provider, cx);
    session.update(cx, |s, cx| {
        s.send_staged(cid("C1"), None, Some("see".into()), cx)
    });
    cx.run_until_parked();
    assert_eq!(uploads(&provider), [(1, Some("see".to_string()))]);
}

#[gpui::test]
async fn staging_uploads_nothing_until_send_and_send_posts_one_message(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    go_online(&provider, cx);
    let dir = tempfile::tempdir().unwrap();
    let paths = vec![
        text_file(dir.path(), "a.txt"),
        text_file(dir.path(), "b.txt"),
    ];
    session.update(cx, |s, cx| s.stage_files(cid("C1"), None, paths, cx));
    cx.run_until_parked();
    assert!(uploads(&provider).is_empty(), "staging does not upload");
    session.read_with(cx, |s, _| {
        let names: Vec<_> = s
            .staged_files(&cid("C1"), None)
            .iter()
            .map(|f| f.name.to_string())
            .collect();
        assert_eq!(names, ["a.txt", "b.txt"]);
    });
    session.update(cx, |s, cx| {
        s.send_staged(cid("C1"), None, Some("two files".into()), cx)
    });
    // In flight: both chips are uploading (no percentage from this provider).
    session.read_with(cx, |s, _| {
        assert!(
            s.staged_files(&cid("C1"), None)
                .iter()
                .all(|f| f.state == StagedState::Uploading(None))
        )
    });
    cx.run_until_parked();
    assert_eq!(uploads(&provider), [(2, Some("two files".to_string()))]);
    session.read_with(cx, |s, _| {
        assert!(s.staged_files(&cid("C1"), None).is_empty())
    });
}

#[gpui::test]
async fn empty_text_with_files_uploads_without_a_comment(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    go_online(&provider, cx);
    let dir = tempfile::tempdir().unwrap();
    let a = text_file(dir.path(), "a.txt");
    session.update(cx, |s, cx| s.stage_files(cid("C1"), None, vec![a], cx));
    session.update(cx, |s, cx| s.send_staged(cid("C1"), None, None, cx));
    cx.run_until_parked();
    assert_eq!(uploads(&provider), [(1, None)]);
}

#[gpui::test]
async fn failed_upload_keeps_chips_with_the_reason_and_retry_resends_the_text(
    cx: &mut TestAppContext,
) {
    let (provider, session) = setup(cx);
    go_online(&provider, cx);
    let dir = tempfile::tempdir().unwrap();
    let a = text_file(dir.path(), "a.txt");
    session.update(cx, |s, cx| s.stage_files(cid("C1"), None, vec![a], cx));
    provider.fail_next(Op::Upload, ProviderError::Network("boom".into()));
    session.update(cx, |s, cx| {
        s.send_staged(cid("C1"), None, Some("hello".into()), cx)
    });
    cx.run_until_parked();
    assert!(uploads(&provider).is_empty());
    let id = session.read_with(cx, |s, _| {
        let f = s.staged_files(&cid("C1"), None);
        match &f[0].state {
            StagedState::Failed(why) => assert!(why.contains("boom"), "{why}"),
            other => panic!("{other:?}"),
        }
        f[0].id
    });
    session.update(cx, |s, cx| s.retry_staged_file(id, cx));
    cx.run_until_parked();
    assert_eq!(uploads(&provider), [(1, Some("hello".to_string()))]);
    session.read_with(cx, |s, _| {
        assert!(s.staged_files(&cid("C1"), None).is_empty())
    });
}

#[gpui::test]
async fn cancel_stops_the_upload_and_retry_sends_it(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    go_online(&provider, cx);
    let dir = tempfile::tempdir().unwrap();
    let a = text_file(dir.path(), "a.txt");
    session.update(cx, |s, cx| s.stage_files(cid("C1"), None, vec![a], cx));
    session.update(cx, |s, cx| {
        s.send_staged(cid("C1"), None, Some("hi".into()), cx)
    });
    let id = session.read_with(cx, |s, _| s.staged_files(&cid("C1"), None)[0].id);
    session.update(cx, |s, cx| s.cancel_staged_file(id, cx));
    cx.run_until_parked();
    assert!(
        uploads(&provider).is_empty(),
        "the upload never reached the provider"
    );
    session.read_with(cx, |s, _| {
        assert_eq!(
            s.staged_files(&cid("C1"), None)[0].state,
            StagedState::Cancelled
        )
    });
    session.update(cx, |s, cx| s.retry_staged_file(id, cx));
    cx.run_until_parked();
    assert_eq!(uploads(&provider), [(1, Some("hi".to_string()))]);
}

#[gpui::test]
async fn staged_files_survive_switching_and_threads_have_their_own(cx: &mut TestAppContext) {
    let (_provider, session) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let a = text_file(dir.path(), "a.txt");
    let t = text_file(dir.path(), "t.txt");
    let root = domain::MessageTs("1.0".into());
    session.update(cx, |s, cx| {
        s.stage_files(cid("C1"), None, vec![a], cx);
        s.stage_files(cid("C1"), Some(root.clone()), vec![t], cx);
    });
    // The session keeps them by draft, not by the selected conversation, so a
    // composer rebuilt after switching away and back finds them again.
    cx.run_until_parked();
    session.read_with(cx, |s, _| {
        assert_eq!(s.staged_files(&cid("C1"), None).len(), 1);
        assert_eq!(s.staged_files(&cid("C1"), Some(&root)).len(), 1);
        assert!(s.staged_files(&cid("C2"), None).is_empty());
    });
}

#[gpui::test]
async fn unstage_removes_the_chip_and_pasted_temp_file(cx: &mut TestAppContext) {
    let (_provider, session) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let up = dir.path().join("uploads");
    std::fs::create_dir_all(&up).unwrap();
    session.update(cx, |s, _| s.set_upload_dir(up.clone()));
    let pasted = text_file(&up, "pasted.png");
    let own = text_file(dir.path(), "own.txt");
    session.update(cx, |s, cx| {
        s.stage_files(cid("C1"), None, vec![pasted.clone(), own.clone()], cx)
    });
    let ids: Vec<u64> = session.read_with(cx, |s, _| {
        s.staged_files(&cid("C1"), None)
            .iter()
            .map(|f| f.id)
            .collect()
    });
    session.update(cx, |s, cx| {
        s.unstage_file(ids[0], cx);
        s.unstage_file(ids[1], cx);
    });
    session.read_with(cx, |s, _| {
        assert!(s.staged_files(&cid("C1"), None).is_empty())
    });
    assert!(!pasted.exists(), "the pasted temp file is deleted");
    assert!(own.exists(), "the user's own file is never touched");
}

#[gpui::test]
async fn oversized_files_are_refused_with_a_notice(cx: &mut TestAppContext) {
    let (_provider, session) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(components::upload::MAX_UPLOAD_BYTES + 1)
        .unwrap();
    session.update(cx, |s, cx| s.stage_files(cid("C1"), None, vec![big], cx));
    session.read_with(cx, |s, _| {
        assert!(s.staged_files(&cid("C1"), None).is_empty());
        assert!(matches!(s.file_notice(), Some(FileNotice::Error(_))));
    });
}

#[gpui::test]
async fn an_expired_session_stages_and_uploads_nothing(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    provider.push_event(ProviderEvent::Connected { reconnect: false });
    cx.run_until_parked();
    let dir = tempfile::tempdir().unwrap();
    let a = text_file(dir.path(), "a.txt");
    let b = text_file(dir.path(), "b.txt");
    session.update(cx, |s, cx| s.stage_files(cid("C1"), None, vec![a], cx));
    provider.push_event(ProviderEvent::SessionExpired);
    cx.run_until_parked();
    // Staging is refused once the session expired.
    session.update(cx, |s, cx| s.stage_files(cid("C1"), None, vec![b], cx));
    session.read_with(cx, |s, _| {
        assert_eq!(s.staged_files(&cid("C1"), None).len(), 1)
    });
    // So is sending what was staged before.
    session.update(cx, |s, cx| s.send_staged(cid("C1"), None, None, cx));
    cx.run_until_parked();
    assert!(uploads(&provider).is_empty());
}

#[gpui::test]
async fn only_small_decodable_images_get_a_thumbnail(cx: &mut TestAppContext) {
    let (_provider, session) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("small.png");
    std::fs::write(&small, b"png").unwrap();
    let huge = dir.path().join("huge.png");
    std::fs::File::create(&huge)
        .unwrap()
        .set_len(21 * 1024 * 1024)
        .unwrap();
    let svg = text_file(dir.path(), "logo.svg");
    let doc = text_file(dir.path(), "a.txt");
    session.update(cx, |s, cx| {
        s.stage_files(cid("C1"), None, vec![small.clone(), huge, svg, doc], cx)
    });
    session.read_with(cx, |s, _| {
        let thumbs: Vec<_> = s
            .staged_files(&cid("C1"), None)
            .iter()
            .map(|f| (f.name.to_string(), f.thumb.is_some()))
            .collect();
        assert_eq!(
            thumbs,
            [
                ("small.png".to_string(), true),
                ("huge.png".to_string(), false),
                ("logo.svg".to_string(), false),
                ("a.txt".to_string(), false),
            ]
        );
    });
}

#[gpui::test]
async fn leaving_or_archiving_a_conversation_drops_its_staged_files(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let up = dir.path().join("uploads");
    std::fs::create_dir_all(&up).unwrap();
    session.update(cx, |s, _| s.set_upload_dir(up.clone()));
    let pasted = text_file(&up, "pasted.png");
    let own = text_file(dir.path(), "own.txt");
    session.update(cx, |s, cx| {
        s.stage_files(cid("C1"), None, vec![pasted.clone(), own.clone()], cx);
        s.stage_files(
            cid("C1"),
            Some(domain::MessageTs("1.0".into())),
            vec![text_file(dir.path(), "t.txt")],
            cx,
        );
    });
    provider.push_event(ProviderEvent::ConversationMetaChanged(
        domain::ConversationMeta {
            id: cid("C1"),
            name: None,
            topic: None,
            is_archived: Some(true),
            is_member: None,
            member_count: None,
            purpose: None,
        },
    ));
    cx.run_until_parked();
    session.read_with(cx, |s, _| {
        assert!(s.staged_files(&cid("C1"), None).is_empty());
        assert!(
            s.staged_files(&cid("C1"), Some(&domain::MessageTs("1.0".into())))
                .is_empty()
        );
    });
    assert!(!pasted.exists(), "its pasted temp file is deleted");
    assert!(own.exists());
}

#[gpui::test]
async fn removing_every_chip_of_a_failed_upload_hands_the_comment_back(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    go_online(&provider, cx);
    let dir = tempfile::tempdir().unwrap();
    let a = text_file(dir.path(), "a.txt");
    let b = text_file(dir.path(), "b.txt");
    session.update(cx, |s, cx| s.stage_files(cid("C1"), None, vec![a, b], cx));
    provider.fail_next(Op::Upload, ProviderError::Network("boom".into()));
    session.update(cx, |s, cx| {
        s.send_staged(cid("C1"), None, Some("hello".into()), cx)
    });
    cx.run_until_parked();
    let ids: Vec<u64> = session.read_with(cx, |s, _| {
        s.staged_files(&cid("C1"), None)
            .iter()
            .map(|f| f.id)
            .collect()
    });
    session.update(cx, |s, cx| s.unstage_file(ids[0], cx));
    session.update(cx, |s, _| {
        assert_eq!(s.take_restored(&cid("C1"), None), None)
    });
    session.update(cx, |s, cx| s.unstage_file(ids[1], cx));
    session.update(cx, |s, _| {
        assert_eq!(s.take_restored(&cid("C1"), None), Some("hello".to_string()));
        assert_eq!(s.take_restored(&cid("C1"), None), None, "handed back once");
    });
}

/// A snippet staged from a long paste uploads as a text file with its language.
#[gpui::test]
async fn a_staged_snippet_uploads_with_its_language(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    go_online(&provider, cx);
    let dir = tempfile::tempdir().unwrap();
    session.update(cx, |s, _| s.set_upload_dir(dir.path().to_path_buf()));
    let snippet = components::NewSnippet {
        name: "contrast_check.rs".into(),
        language: "rust".into(),
        text: "fn a() {}\nfn b() {}\n".into(),
        as_file: false,
    };
    session.update(cx, |s, cx| s.stage_snippet(cid("C1"), None, snippet, cx));
    // The text is written to its temp file off the main thread.
    cx.run_until_parked();
    session.read_with(cx, |s, _| {
        let staged = s.staged_files(&cid("C1"), None);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].name.as_ref(), "contrast_check.rs");
        let info = staged[0].snippet.clone().expect("a snippet chip");
        // The id: the composer names it in the UI language.
        assert_eq!((info.language.as_ref(), info.lines), ("rust", 2));
    });
    session.update(cx, |s, cx| s.send_staged(cid("C1"), None, None, cx));
    cx.run_until_parked();
    let sent: Vec<_> = provider
        .calls()
        .iter()
        .filter_map(|c| match c {
            Call::Upload { files, .. } => Some(files.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    let f = &sent[0][0];
    assert_eq!(&*f.name, "contrast_check.rs");
    assert_eq!(f.filetype.as_deref(), Some("rust"));
    assert_eq!(&*f.mime, "text/plain");
    assert_eq!(f.size, 20);
}

/// A snippet and a plain file staged together: only the snippet carries a language.
#[gpui::test]
async fn an_ordinary_file_has_no_language(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    go_online(&provider, cx);
    let dir = tempfile::tempdir().unwrap();
    let a = text_file(dir.path(), "a.txt");
    session.update(cx, |s, cx| s.stage_files(cid("C1"), None, vec![a], cx));
    session.update(cx, |s, cx| s.send_staged(cid("C1"), None, None, cx));
    cx.run_until_parked();
    let langs: Vec<_> = provider
        .calls()
        .iter()
        .filter_map(|c| match c {
            Call::Upload { files, .. } => Some(files[0].filetype.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(langs, [None]);
}

/// An oversized paste sent as a file is an ordinary text file, not a snippet.
#[gpui::test]
async fn a_paste_sent_as_a_file_has_no_language(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    go_online(&provider, cx);
    let dir = tempfile::tempdir().unwrap();
    session.update(cx, |s, _| s.set_upload_dir(dir.path().to_path_buf()));
    let paste = components::NewSnippet {
        name: "paste.txt".into(),
        language: "text".into(),
        text: "x\n".repeat(10),
        as_file: true,
    };
    session.update(cx, |s, cx| s.stage_snippet(cid("C1"), None, paste, cx));
    cx.run_until_parked();
    session.read_with(cx, |s, _| {
        let staged = s.staged_files(&cid("C1"), None);
        assert_eq!(staged.len(), 1);
        assert!(staged[0].snippet.is_none(), "a plain file chip");
    });
    session.update(cx, |s, cx| s.send_staged(cid("C1"), None, None, cx));
    cx.run_until_parked();
    let langs: Vec<_> = provider
        .calls()
        .iter()
        .filter_map(|c| match c {
            Call::Upload { files, .. } => Some(files[0].filetype.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(langs, [None]);
}

fn named(url: &RemoteUrl, name: &str, mime: &str) -> FileRef {
    let mut f = file(url);
    f.name = Arc::from(name);
    f.mime = Arc::from(mime);
    f
}

type Opened = std::rc::Rc<std::cell::RefCell<Vec<std::path::PathBuf>>>;

fn recording_opener(session: &Entity<WorkspaceSession>, cx: &mut TestAppContext) -> Opened {
    let opened: Opened = Default::default();
    let o = opened.clone();
    session.update(cx, |s, _| {
        s.set_opener(std::rc::Rc::new(move |p: &std::path::Path, _| {
            o.borrow_mut().push(p.to_path_buf())
        }))
    });
    opened
}

#[test]
fn only_known_image_and_pdf_files_may_open_in_the_default_app() {
    use parla::files::may_open;
    assert!(may_open("report.pdf", "application/pdf"));
    assert!(may_open("cat.JPG", "image/jpeg"));
    assert!(may_open("shot.heic", "image/heic"));
    assert!(
        !may_open("evil.command", "application/pdf"),
        "extension decides too"
    );
    assert!(
        !may_open("report.pdf", "application/x-sh"),
        "mime decides too"
    );
    assert!(!may_open("logo.svg", "image/svg+xml"), "svg can run script");
    assert!(
        !may_open("cat.png", "application/pdf"),
        "extension and mime agree"
    );
    assert!(!may_open("noext", "image/png"));
}

#[gpui::test]
async fn open_in_default_app_opens_an_allowed_file_once_and_never_a_disguised_one(
    cx: &mut TestAppContext,
) {
    let (provider, session) = setup(cx);
    let url = RemoteUrl(Arc::from("https://files.example/F1/x"));
    provider.add_file(url.clone(), b"pdf".to_vec());
    let dir = tempfile::tempdir().unwrap();
    session.update(cx, |s, _| s.set_download_dir(dir.path().to_path_buf()));
    let opened = recording_opener(&session, cx);
    let pdf = named(&url, "report.pdf", "application/pdf");
    // A second press while the first download runs starts nothing.
    session.update(cx, |s, cx| s.download_and_open(&pdf, cx));
    session.update(cx, |s, cx| s.download_and_open(&pdf, cx));
    cx.run_until_parked();
    assert_eq!(*opened.borrow(), [dir.path().join("report.pdf")]);
    let downloads = || {
        provider
            .calls()
            .iter()
            .filter(|c| matches!(c, Call::Download { .. }))
            .count()
    };
    assert_eq!(downloads(), 1, "one download at a time per file");
    for (name, mime) in [
        ("evil.command", "application/pdf"),
        ("logo.svg", "image/svg+xml"),
    ] {
        let f = named(&url, name, mime);
        session.update(cx, |s, cx| s.download_and_open(&f, cx));
        cx.run_until_parked();
        assert!(dir.path().join(name).exists(), "{name} is saved");
    }
    assert_eq!(opened.borrow().len(), 1, "a disguised file is never opened");
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[gpui::test]
async fn every_download_is_marked_as_from_the_internet(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    let url = RemoteUrl(Arc::from("https://files.example/F1/report.pdf"));
    provider.add_file(url.clone(), b"pdf".to_vec());
    let dir = tempfile::tempdir().unwrap();
    session.update(cx, |s, _| s.set_download_dir(dir.path().to_path_buf()));
    let f = file(&url);
    session.update(cx, |s, cx| s.download(&f, cx));
    cx.run_until_parked();
    assert!(parla::quarantine::is_marked(&dir.path().join("report.pdf")));
}
