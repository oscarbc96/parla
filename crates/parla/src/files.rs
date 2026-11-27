//! File downloads and uploads of a `WorkspaceSession`. Downloads go through
//! `ChatProvider::download` (host-checked) into the downloads folder and are
//! opened only by the media viewer's explicit "Open in default app". Files picked, dropped or pasted are staged on the draft of a
//! conversation or thread (in memory, so they survive switching away) and
//! upload on Send as one message with the text as its comment; uploads need a
//! live connection and are not queued. The provider reports only done or
//! failed, so a chip shows no percentage.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use components::upload::{cleanup_temp, plan, uploads_dir};
use components::{FileNotice, StagedFile, StagedState};
use domain::{Connection, ConversationId, FileRef, Input, MessageTs, ProviderEvent, Upload};
use futures::future::{AbortHandle, abortable};
use gpui::{App, AppContext as _, Context};
use std::rc::Rc;

use crate::session::{WorkspaceSession, new_client_id};

/// The draft a staged file belongs to: a conversation, or one of its threads.
type StageKey = (ConversationId, Option<MessageTs>);

/// A file attached to a draft.
struct Staged {
    key: StageKey,
    id: u64,
    upload: Upload,
    /// Lines of a snippet's text (0 for any other file), for its chip.
    lines: usize,
    /// Set once the file belongs to an upload (a Send): its files go out together.
    batch: Option<u64>,
    state: StagedState,
}

/// One Send's upload: the comment it carries (kept for Retry) and how to cancel it.
struct Batch {
    text: Option<String>,
    abort: Option<AbortHandle>,
    /// Counts the starts of this upload (Send, then each Retry); a result of an older one is stale.
    attempt: u64,
}

/// Images above this size show the type badge instead of a thumbnail (decoding costs memory).
const THUMB_MAX_BYTES: u64 = 20 * 1024 * 1024;

/// Image types the thumbnail decoder reads.
fn thumbnailable(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

/// Opens a saved file in the system's default app (replaced in tests).
pub type Opener = Rc<dyn Fn(&Path, &mut App)>;

/// Files "Open in default app" may hand to the system: these extensions, each
/// with its own type. Anything else (an SVG, a script named `.pdf`) is only
/// saved.
const OPENABLE: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("heic", "image/heic"),
    ("pdf", "application/pdf"),
];

/// `name` (by its extension) and `mime` both say a file "Open in default app"
/// may open.
pub fn may_open(name: &str, mime: &str) -> bool {
    let Some(ext) = Path::new(name).extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    let mime = mime.trim().to_ascii_lowercase();
    OPENABLE.iter().any(|(e, m)| *e == ext && *m == mime)
}

#[derive(Default)]
pub(crate) struct FileState {
    /// Files being saved for "Open in default app": one at a time per file.
    opening: HashSet<domain::FileId>,
    opener: Option<Opener>,
    download_dir: Option<PathBuf>,
    upload_dir: Option<PathBuf>,
    reserved: HashSet<PathBuf>,
    staged: Vec<Staged>,
    batches: HashMap<u64, Batch>,
    /// Comments of removed failed uploads, waiting to go back into their draft.
    restored: HashMap<StageKey, String>,
    next_id: u64,
    pub(crate) notice: Option<FileNotice>,
}

impl FileState {
    pub(crate) fn staged_in(
        &self,
        c: &ConversationId,
        thread: Option<&MessageTs>,
    ) -> Vec<StagedFile> {
        self.staged
            .iter()
            .filter(|s| &s.key.0 == c && s.key.1.as_ref() == thread)
            .map(|s| StagedFile {
                id: s.id,
                name: s.upload.name.to_string().into(),
                size: s.upload.size,
                thumb: (thumbnailable(&s.upload.mime) && s.upload.size <= THUMB_MAX_BYTES)
                    .then(|| s.upload.path.clone()),
                snippet: s
                    .upload
                    .filetype
                    .as_ref()
                    .map(|t| components::StagedSnippet {
                        // The id: the composer names it in the UI language.
                        language: t.to_string().into(),
                        lines: s.lines,
                    }),
                state: s.state.clone(),
            })
            .collect()
    }
}

/// `name.ext` -> `name (1).ext`, `name (2).ext`, ... until the name is free.
pub fn unique_path(dir: &Path, name: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    // Never let a provider-supplied name escape the folder.
    let name = Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "download".into());
    let first = dir.join(&name);
    if !taken(&first) {
        return first;
    }
    let p = Path::new(&name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = p.extension().map(|e| e.to_string_lossy().into_owned());
    (1..)
        .map(|n| match &ext {
            Some(e) => dir.join(format!("{stem} ({n}).{e}")),
            None => dir.join(format!("{stem} ({n})")),
        })
        .find(|c| !taken(c))
        .expect("unbounded range")
}

impl WorkspaceSession {
    pub fn set_download_dir(&mut self, dir: PathBuf) {
        self.files.download_dir = Some(dir);
    }

    pub fn set_upload_dir(&mut self, dir: PathBuf) {
        self.files.upload_dir = Some(dir);
    }

    /// Latest download or upload result.
    pub fn file_notice(&self) -> Option<FileNotice> {
        self.files.notice.clone()
    }

    fn file_error(&mut self, msg: i18n::LocalizedText, cx: &mut Context<Self>) {
        self.files.notice = Some(FileNotice::Error(msg));
        cx.notify();
    }

    /// Saves `file` into the downloads folder under a unique name. Never opens it.
    pub fn download(&mut self, file: &FileRef, cx: &mut Context<Self>) {
        self.save_file(file, false, cx);
    }

    /// Saves `file` like [`Self::download`], then opens it in the system's
    /// default app: only on the user's explicit "Open in default app", and
    /// only a file [`may_open`] allows (anything else is just saved). A second
    /// press while the file is still downloading does nothing.
    pub fn download_and_open(&mut self, file: &FileRef, cx: &mut Context<Self>) {
        if file.download.is_some() && !self.files.opening.insert(file.id.clone()) {
            return;
        }
        self.save_file(file, may_open(&file.name, &file.mime), cx);
    }

    /// Replaces how saved files open in the default app (tests).
    pub fn set_opener(&mut self, opener: Opener) {
        self.files.opener = Some(opener);
    }

    fn save_file(&mut self, file: &FileRef, open: bool, cx: &mut Context<Self>) {
        let Some(url) = file.download.clone() else {
            return self.file_error(
                i18n::LocalizedText::new("parla", "file-error-no-download"),
                cx,
            );
        };
        let dir = self
            .files
            .download_dir
            .clone()
            .or_else(dirs::download_dir)
            .unwrap_or_else(std::env::temp_dir);
        let reserved = &self.files.reserved;
        let dest = unique_path(&dir, &file.name, |p| p.exists() || reserved.contains(p));
        self.files.reserved.insert(dest.clone());
        let p = self.provider.clone();
        let d = dest.clone();
        let id = file.id.clone();
        let now = components::now_secs(cx);
        self.run(
            cx,
            async move {
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let n = p.download(&url, &d).await.map_err(|e| e.to_string())?;
                // The system asks before running or opening it.
                if let Err(e) = crate::quarantine::mark(&d, now) {
                    crate::log::warn(&format!("could not mark a download: {:?}", e.kind()));
                }
                Ok(n)
            },
            move |s, r: Result<u64, String>, cx| {
                s.files.reserved.remove(&dest);
                s.files.opening.remove(&id);
                match r {
                    Ok(_) => {
                        if open {
                            match s.files.opener.clone() {
                                Some(f) => f(&dest, cx),
                                None => cx.open_with_system(&dest),
                            }
                        }
                        s.files.notice = Some(FileNotice::Downloaded(dest));
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&dest);
                        s.files.notice = Some(FileNotice::Error(
                            i18n::LocalizedText::new("parla", "file-error-download-failed")
                                .with("reason", e),
                        ));
                    }
                }
                cx.notify();
            },
        );
    }

    /// Fetches a clip rendition into the clip cache (host-checked, never opened by
    /// the shell) and reports to `done`. Old clips are pruned to the cache cap.
    pub(crate) fn fetch_clip(
        &mut self,
        url: domain::RemoteUrl,
        dest: PathBuf,
        done: components::timeline::ClipDone,
        cx: &mut Context<Self>,
    ) {
        let p = self.provider.clone();
        self.run(
            cx,
            async move {
                let dir = dest.parent().ok_or("bad cache path")?.to_path_buf();
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let part = dest.with_extension("part");
                if let Err(e) = p.download(&url, &part).await {
                    let _ = std::fs::remove_file(&part);
                    return Err(e.to_string());
                }
                std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
                components::media::prune_disk_cache(&dir, components::media::DISK_CAP_BYTES);
                Ok::<(), String>(())
            },
            move |_, r, cx| done(r, cx),
        );
    }

    /// Fetches a clip's captions through the provider (host-checked), stopping
    /// past `max_bytes`, into memory only: nothing is cached on disk, so a
    /// refused file leaves nothing behind.
    pub(crate) fn fetch_captions(
        &mut self,
        url: domain::RemoteUrl,
        max_bytes: u64,
        done: components::timeline::CaptionsDone,
        cx: &mut Context<Self>,
    ) {
        let p = self.provider.clone();
        self.run(
            cx,
            async move { p.fetch(&url, max_bytes).await.map_err(|e| e.to_string()) },
            move |_, r, cx| done(r, cx),
        );
    }

    fn upload_temp_dir(&self) -> PathBuf {
        self.files.upload_dir.clone().unwrap_or_else(uploads_dir)
    }

    /// Attaches local files to the draft of `c` (or its thread). Nothing leaves the
    /// machine until [`Self::send_staged`]. Files over the size limit are refused with a notice.
    pub fn stage_files(
        &mut self,
        c: ConversationId,
        thread: Option<MessageTs>,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let temp_dir = self.upload_temp_dir();
        // A signed-out session is read-only: drag and drop, paste and attach
        // all end here.
        if self.state.session == domain::Session::Expired {
            cleanup_temp(&paths, &temp_dir);
            return;
        }
        let planned = plan(&paths);
        if !planned.refused.is_empty() {
            let reasons =
                components::upload::refusals_text(&planned.refused, &i18n::gpui::localizer(cx));
            self.files.notice = Some(FileNotice::Error(
                i18n::LocalizedText::new("parla", "file-error-refused").with("reasons", reasons),
            ));
        }
        if planned.accepted.is_empty() {
            cleanup_temp(&paths, &temp_dir);
        }
        for upload in planned.accepted {
            self.files.next_id += 1;
            self.files.staged.push(Staged {
                key: (c.clone(), thread.clone()),
                id: self.files.next_id,
                upload,
                lines: 0,
                batch: None,
                state: StagedState::Ready,
            });
        }
        self.staged_changed(cx);
    }

    /// Attaches a snippet created from a long paste: its text is written to a temp file on
    /// the background executor and uploads on Send as a text file of `snippet.language` (a
    /// snippet to the provider), or as an ordinary text file when `snippet.as_file`.
    pub fn stage_snippet(
        &mut self,
        c: ConversationId,
        thread: Option<MessageTs>,
        snippet: components::NewSnippet,
        cx: &mut Context<Self>,
    ) {
        if self.state.session == domain::Session::Expired {
            return;
        }
        let dir = self.upload_temp_dir();
        let language = components::snippet::language_by_id(&snippet.language)
            .filter(|_| !snippet.as_file)
            .unwrap_or_else(components::snippet::plain);
        let ext = language.ext;
        let lines = snippet.text.lines().count().max(1);
        let components::NewSnippet {
            name,
            text,
            as_file,
            ..
        } = snippet;
        let write = cx.background_spawn({
            let dir = dir.clone();
            async move {
                let path = components::upload::write_snippet(&dir, &text, ext)?;
                let planned = plan(std::slice::from_ref(&path));
                Ok::<_, std::io::Error>((path, planned))
            }
        });
        cx.spawn(async move |this, cx| {
            let written = write.await;
            this.update(cx, |s, cx| {
                let (path, mut planned) = match written {
                    Ok(w) => w,
                    Err(e) => {
                        return s.file_error(
                            i18n::LocalizedText::new("parla", "file-error-refused")
                                .with("reasons", e.to_string()),
                            cx,
                        );
                    }
                };
                if let Some(refusal) = planned.refused.first() {
                    let reasons = components::upload::refusals_text(
                        std::slice::from_ref(refusal),
                        &i18n::gpui::localizer(cx),
                    );
                    cleanup_temp(&[path], &dir);
                    return s.file_error(
                        i18n::LocalizedText::new("parla", "file-error-refused")
                            .with("reasons", reasons),
                        cx,
                    );
                }
                let Some(mut upload) = planned.accepted.pop() else {
                    cleanup_temp(&[path], &dir);
                    return;
                };
                upload.name = name.as_str().into();
                upload.mime = "text/plain".into();
                upload.filetype = (!as_file).then(|| language.id.into());
                s.files.next_id += 1;
                s.files.staged.push(Staged {
                    key: (c, thread),
                    id: s.files.next_id,
                    upload,
                    lines,
                    batch: None,
                    state: StagedState::Ready,
                });
                s.staged_changed(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Files staged on a draft, oldest first.
    pub fn staged_files(&self, c: &ConversationId, thread: Option<&MessageTs>) -> Vec<StagedFile> {
        self.files.staged_in(c, thread)
    }

    /// Files can be uploaded now: online and signed in.
    pub fn can_upload_files(&self) -> bool {
        self.state.connection == Connection::Online
            && self.state.session != domain::Session::Expired
    }

    fn staged_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// The comment of a failed upload whose files were all removed, once, so it can go back
    /// into the draft it was sent from.
    pub fn take_restored(
        &mut self,
        c: &ConversationId,
        thread: Option<&MessageTs>,
    ) -> Option<String> {
        if self.files.restored.is_empty() {
            return None;
        }
        self.files.restored.remove(&(c.clone(), thread.cloned()))
    }

    /// Detaches a staged file (an uploading one is cancelled first). When it was the last of a
    /// failed or cancelled upload, the comment that upload carried goes back to the draft.
    pub fn unstage_file(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(i) = self.files.staged.iter().position(|s| s.id == id) else {
            return;
        };
        if matches!(self.files.staged[i].state, StagedState::Uploading(_)) {
            self.cancel_staged_file(id, cx);
        }
        let Some(i) = self.files.staged.iter().position(|s| s.id == id) else {
            return;
        };
        let gone = self.files.staged.remove(i);
        cleanup_temp(&[gone.upload.path], &self.upload_temp_dir());
        if let Some(b) = gone.batch
            && !self.files.staged.iter().any(|s| s.batch == Some(b))
            && let Some(batch) = self.files.batches.remove(&b)
        {
            if let Some(h) = batch.abort {
                h.abort();
            }
            if let Some(text) = batch.text.filter(|t| !t.trim().is_empty()) {
                self.files
                    .restored
                    .entry(gone.key)
                    .and_modify(|t| *t = [t.as_str(), text.as_str()].join("\n"))
                    .or_insert(text);
            }
        }
        self.staged_changed(cx);
    }

    /// Cancels the upload `id` is part of: its files stay staged as cancelled and
    /// the comment is kept for Retry.
    pub fn cancel_staged_file(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(batch) = self
            .files
            .staged
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.batch)
        else {
            return;
        };
        if let Some(h) = self
            .files
            .batches
            .get_mut(&batch)
            .and_then(|b| b.abort.take())
        {
            h.abort();
        }
        for s in self
            .files
            .staged
            .iter_mut()
            .filter(|s| s.batch == Some(batch))
        {
            if matches!(s.state, StagedState::Uploading(_)) {
                s.state = StagedState::Cancelled;
            }
        }
        self.staged_changed(cx);
    }

    /// Drops everything staged for a conversation and its threads: it was left or archived.
    pub(crate) fn prune_conversation(&mut self, c: &ConversationId, cx: &mut Context<Self>) {
        if !self.files.staged.iter().any(|s| &s.key.0 == c) {
            self.files.restored.retain(|k, _| &k.0 != c);
            return;
        }
        let temp_dir = self.upload_temp_dir();
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.files.staged)
            .into_iter()
            .partition(|s| &s.key.0 == c);
        self.files.staged = kept;
        for s in &gone {
            if let Some(b) = s.batch.and_then(|b| self.files.batches.remove(&b))
                && let Some(h) = b.abort
            {
                h.abort();
            }
        }
        let paths: Vec<PathBuf> = gone.into_iter().map(|s| s.upload.path).collect();
        cleanup_temp(&paths, &temp_dir);
        self.files.restored.retain(|k, _| &k.0 != c);
        self.staged_changed(cx);
    }

    /// Uploads the ready files of a draft as one message, `text` as its comment (the
    /// text may be empty). Clears the saved draft text. Only while online and signed in.
    pub fn send_staged(
        &mut self,
        c: ConversationId,
        thread: Option<MessageTs>,
        text: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.state.session == domain::Session::Expired {
            return;
        }
        if self.state.connection != Connection::Online {
            return self.file_error(
                i18n::LocalizedText::new("parla", "file-error-needs-connection"),
                cx,
            );
        }
        self.files.next_id += 1;
        let batch = self.files.next_id;
        let key: StageKey = (c.clone(), thread.clone());
        let mut any = false;
        for s in self.files.staged.iter_mut() {
            if s.key == key && s.state == StagedState::Ready {
                s.batch = Some(batch);
                any = true;
            }
        }
        if !any {
            return;
        }
        self.set_draft(
            domain::DraftKey {
                conversation: c,
                thread,
            },
            "",
            cx,
        );
        self.files.batches.insert(
            batch,
            Batch {
                text,
                abort: None,
                attempt: 0,
            },
        );
        self.start_batch(batch, cx);
    }

    /// Sends the upload `id` belonged to again.
    pub fn retry_staged_file(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(batch) = self
            .files
            .staged
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.batch)
        else {
            return;
        };
        if !self.can_upload_files() {
            if self.state.session != domain::Session::Expired {
                self.file_error(
                    i18n::LocalizedText::new("parla", "file-error-needs-connection"),
                    cx,
                );
            }
            return;
        }
        self.start_batch(batch, cx);
    }

    /// Starts (or restarts) the upload of `batch`. Nothing changes when a file of it is
    /// already going out.
    fn start_batch(&mut self, batch: u64, cx: &mut Context<Self>) {
        let mine = |s: &&Staged| s.batch == Some(batch);
        if self
            .files
            .staged
            .iter()
            .filter(mine)
            .any(|s| matches!(s.state, StagedState::Uploading(_)))
        {
            return;
        }
        let mut key = None;
        let mut uploads = Vec::new();
        for s in self
            .files
            .staged
            .iter_mut()
            .filter(|s| s.batch == Some(batch))
        {
            s.state = StagedState::Uploading(None);
            key = Some(s.key.clone());
            uploads.push(s.upload.clone());
        }
        let Some((conv, thread)) = key else {
            self.files.batches.remove(&batch);
            return;
        };
        let p = self.provider.clone();
        let client_id = new_client_id(cx);
        let Some(b) = self.files.batches.get_mut(&batch) else {
            return;
        };
        b.attempt += 1;
        let attempt = b.attempt;
        let text = b.text.clone();
        let (fut, abort) = abortable(async move {
            p.upload(&conv, thread, uploads, text.as_deref(), client_id)
                .await
        });
        b.abort = Some(abort);
        self.staged_changed(cx);
        self.run_or(
            cx,
            fut,
            move |s, r, cx| s.upload_finished(batch, attempt, r.ok(), cx),
            move |s, cx| {
                s.upload_finished(
                    batch,
                    attempt,
                    Some(Err(domain::ProviderError::Other("task-lost".into()))),
                    cx,
                )
            },
        );
    }

    /// `r` is `None` when the attempt was cancelled (its chips already say so). A failure of an
    /// attempt that was since replaced is ignored; a success of any attempt ends the batch,
    /// because the message is posted, and stops the attempt in flight so it is not posted twice.
    fn upload_finished(
        &mut self,
        batch: u64,
        attempt: u64,
        r: Option<Result<domain::Message, domain::ProviderError>>,
        cx: &mut Context<Self>,
    ) {
        let (Some(r), Some(current)) = (r, self.files.batches.get(&batch).map(|b| b.attempt))
        else {
            return;
        };
        match r {
            Ok(m) => {
                if let Some(b) = self.files.batches.remove(&batch)
                    && let Some(h) = b.abort
                {
                    h.abort();
                }
                let temp_dir = self.upload_temp_dir();
                let (done, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.files.staged)
                    .into_iter()
                    .partition(|s| s.batch == Some(batch));
                self.files.staged = rest;
                let paths: Vec<PathBuf> = done.into_iter().map(|s| s.upload.path).collect();
                cleanup_temp(&paths, &temp_dir);
                self.apply(Input::Provider(ProviderEvent::MessagePosted(m)), cx);
            }
            Err(_) if attempt != current => return,
            Err(e) => {
                if let Some(b) = self.files.batches.get_mut(&batch) {
                    b.abort = None;
                }
                let why: gpui::SharedString = i18n::gpui::localizer(cx)
                    .text(&crate::error_text::provider_error(&e))
                    .into_owned()
                    .into();
                for s in self
                    .files
                    .staged
                    .iter_mut()
                    .filter(|s| s.batch == Some(batch))
                {
                    s.state = StagedState::Failed(why.clone());
                }
            }
        }
        self.staged_changed(cx);
    }
}

/// The Files tab reads the files of the messages the store holds for the
/// conversation (bounded pages, newest first). A provider-backed list is a
/// follow-up: no provider documents a per-conversation file list yet.
impl components::FilesSource for WorkspaceSession {
    fn files_state(&self) -> &domain::WorkspaceState {
        self.state()
    }

    fn load_files(
        &mut self,
        c: ConversationId,
        cursor: Option<std::sync::Arc<str>>,
        limit: usize,
        done: components::FilesDone,
        cx: &mut App,
    ) {
        let st = self.store_handle();
        let ws = self.state().workspace.clone();
        self.defer_to(cx, move |s, cx| {
            // Exactly one of the two endings answers.
            let done = Rc::new(std::cell::RefCell::new(Some(done)));
            let lost = done.clone();
            s.run_or(
                cx,
                async move {
                    st.conversation_files(&ws, &c, cursor.map(MessageTs), limit)
                        .await
                        .map_err(|e| e.to_string())
                },
                move |_, r, cx| {
                    if let Some(d) = done.borrow_mut().take() {
                        d(r, cx);
                    }
                },
                move |_, cx| {
                    if let Some(d) = lost.borrow_mut().take() {
                        d(Err("lost".into()), cx);
                    }
                },
            );
        });
    }

    fn view_file(&mut self, gallery: components::media_viewer::Gallery, cx: &mut App) {
        components::TimelineSource::open_media(self, gallery, cx);
    }

    fn view_code(&mut self, gallery: components::media_viewer::Gallery, cx: &mut App) {
        components::TimelineSource::open_code(self, gallery, cx);
    }

    fn download_file(&mut self, file: &FileRef, cx: &mut App) {
        components::TimelineSource::download(self, file, cx);
    }

    fn files_inline_images(&self, cx: &App) -> settings::InlineImages {
        self.settings(cx).messages.inline_images
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use domain::test_support::{FakeProvider, cid, conv, member, uid};
    use domain::{ChatProvider, ProviderEvent};
    use gpui::TestAppContext;
    use store::{MemoryStore, Store};

    use super::*;

    fn session(cx: &mut TestAppContext) -> (Arc<FakeProvider>, gpui::Entity<WorkspaceSession>) {
        cx.update(|cx| {
            i18n::gpui::init(
                cx,
                &[components::catalog(), crate::catalog()],
                i18n::LocalePref::Code("en".parse().unwrap()),
            );
        });
        let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
        provider.add_conversation(conv("C1", "general"));
        provider.add_member(member("u1", "Me"));
        let p: Arc<dyn ChatProvider> = provider.clone();
        let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
        let s = cx.new(|cx| WorkspaceSession::new(p, st, cx));
        cx.run_until_parked();
        provider.push_event(ProviderEvent::Connected { reconnect: false });
        cx.run_until_parked();
        (provider, s)
    }

    fn stage_one(
        s: &mut WorkspaceSession,
        dir: &Path,
        name: &str,
        cx: &mut Context<WorkspaceSession>,
    ) {
        let p = dir.join(name);
        std::fs::write(&p, b"hi").unwrap();
        s.stage_files(cid("C1"), None, vec![p], cx);
    }

    /// A result of an attempt that was replaced (cancel, then Retry) says nothing about the
    /// attempt in flight: it must not flip the chips to failed.
    #[gpui::test]
    async fn a_stale_failure_does_not_touch_the_attempt_in_flight(cx: &mut TestAppContext) {
        let (_p, session) = session(cx);
        let dir = tempfile::tempdir().unwrap();
        session.update(cx, |s, cx| {
            stage_one(s, dir.path(), "a.txt", cx);
            s.send_staged(cid("C1"), None, Some("x".into()), cx);
            let id = s.files.staged[0].id;
            s.cancel_staged_file(id, cx);
            s.retry_staged_file(id, cx);
            let batch = s.files.staged[0].batch.unwrap();
            // The first attempt's failure arrives late.
            s.upload_finished(
                batch,
                1,
                Some(Err(domain::ProviderError::Network("late".into()))),
                cx,
            );
            assert_eq!(s.files.staged[0].state, StagedState::Uploading(None));
        });
    }

    /// A late success of a replaced attempt still ends the batch: the message is posted,
    /// so the retry in flight must not post it again.
    #[gpui::test]
    async fn a_stale_success_ends_the_batch_and_stops_the_retry(cx: &mut TestAppContext) {
        let (provider, session) = session(cx);
        let dir = tempfile::tempdir().unwrap();
        let msg = domain::test_support::msg("C1", "9.0", "x");
        session.update(cx, |s, cx| {
            stage_one(s, dir.path(), "a.txt", cx);
            s.send_staged(cid("C1"), None, Some("x".into()), cx);
            let id = s.files.staged[0].id;
            s.cancel_staged_file(id, cx);
            s.retry_staged_file(id, cx);
            let batch = s.files.staged[0].batch.unwrap();
            s.upload_finished(batch, 1, Some(Ok(msg)), cx);
            assert!(s.files.staged.is_empty());
        });
        cx.run_until_parked();
        let uploads = provider
            .calls()
            .iter()
            .filter(|c| matches!(c, domain::test_support::Call::Upload { .. }))
            .count();
        assert_eq!(
            uploads, 0,
            "the retry was aborted before it reached the provider"
        );
    }

    /// Retry on a batch that has a file already going out changes nothing.
    #[gpui::test]
    async fn start_batch_marks_nothing_when_a_file_is_already_uploading(cx: &mut TestAppContext) {
        let (_p, session) = session(cx);
        let dir = tempfile::tempdir().unwrap();
        session.update(cx, |s, cx| {
            stage_one(s, dir.path(), "a.txt", cx);
            stage_one(s, dir.path(), "b.txt", cx);
            s.send_staged(cid("C1"), None, None, cx);
            let batch = s.files.staged[0].batch.unwrap();
            s.files.staged[0].state = StagedState::Failed("x".into());
            s.start_batch(batch, cx);
            assert_eq!(
                s.files.staged[0].state,
                StagedState::Failed("x".into()),
                "the failed file is not marked uploading with no task behind it"
            );
        });
    }
}
