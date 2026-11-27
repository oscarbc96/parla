//! `CallController`: spawns the call helper, feeds the state machine with its
//! events and executes the effects. Event driven: the only timers are the 30 s
//! load timer (between spawn and the first load) and the 2 s leave grace.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use domain::ChatProvider;
use futures::future::BoxFuture;
use futures::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use futures::{FutureExt, StreamExt, channel::mpsc};
use gpui::{AppContext as _, AsyncApp, Context, EventEmitter, Task, WeakEntity};
use settings::CallSettings;
use zeroize::Zeroizing;

use crate::protocol::{MAX_LINE, ParentMsg, WireSurface, decode_helper, encode_parent};
use crate::session::{CallEffect, CallInput, CallKey, CallState, FailReason, step};

/// A load that takes longer than this offers "Open in browser".
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the helper gets to exit after `Leave` before it is killed.
pub const LEAVE_GRACE: Duration = Duration::from_secs(2);

pub struct HelperProcess {
    pub stdin: Box<dyn AsyncWrite + Send + Unpin>,
    pub stdout: Box<dyn AsyncBufRead + Send + Unpin>,
    pub kill: Box<dyn FnOnce() + Send>,
    pub exited: BoxFuture<'static, ()>,
}

pub trait HelperSpawner: 'static {
    fn spawn(&self) -> io::Result<HelperProcess>;
}

/// Runs this executable as `parla call-window`. Nothing is added to argv or the
/// environment; the surface (and the cookie in it) goes over stdin only.
#[derive(Default)]
pub struct ExeSpawner {
    rt: Option<tokio::runtime::Handle>,
}

impl ExeSpawner {
    /// The shared Tokio runtime that owns the child process.
    pub fn new(rt: tokio::runtime::Handle) -> Self {
        Self { rt: Some(rt) }
    }

    pub fn command(&self) -> io::Result<std::process::Command> {
        let mut c = std::process::Command::new(std::env::current_exe()?);
        c.arg("call-window")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());
        Ok(c)
    }
}

impl HelperSpawner for ExeSpawner {
    fn spawn(&self) -> io::Result<HelperProcess> {
        use tokio_util_compat::{ChildStdinCompat, ChildStdoutCompat};
        let rt = self
            .rt
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())
            .ok_or_else(|| io::Error::other("no tokio runtime"))?;
        let _guard = rt.enter();
        let mut child = tokio::process::Command::from(self.command()?)
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no stdout"))?;
        let (kill_tx, kill_rx) = futures::channel::oneshot::channel::<()>();
        let (done_tx, done_rx) = futures::channel::oneshot::channel::<()>();
        // The child is owned by a task on the shared runtime: it waits for the
        // exit or a kill request, so no thread polls.
        rt.spawn(async move {
            tokio::select! {
                _ = child.wait() => {}
                _ = kill_rx => { let _ = child.kill().await; }
            }
            let _ = done_tx.send(());
        });
        Ok(HelperProcess {
            stdin: Box::new(ChildStdinCompat::new(stdin)),
            stdout: Box::new(ChildStdoutCompat::new(stdout)),
            kill: Box::new(move || {
                let _ = kill_tx.send(());
            }),
            exited: done_rx.map(|_| ()).boxed(),
        })
    }
}

/// Minimal tokio <-> futures io adapters (no extra dependency).
mod tokio_util_compat {
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use futures::io::{AsyncBufRead, AsyncRead, AsyncWrite};
    use tokio::io::AsyncRead as _;
    use tokio::io::AsyncWrite as _;

    pub struct ChildStdinCompat(tokio::process::ChildStdin);
    impl ChildStdinCompat {
        pub fn new(s: tokio::process::ChildStdin) -> Self {
            Self(s)
        }
    }
    impl AsyncWrite for ChildStdinCompat {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            b: &[u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.0).poll_write(cx, b)
        }
        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.0).poll_flush(cx)
        }
        fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.0).poll_shutdown(cx)
        }
    }

    pub struct ChildStdoutCompat {
        inner: tokio::process::ChildStdout,
        buf: Box<[u8]>,
        pos: usize,
        cap: usize,
    }
    impl ChildStdoutCompat {
        pub fn new(s: tokio::process::ChildStdout) -> Self {
            Self {
                inner: s,
                buf: vec![0; 8192].into_boxed_slice(),
                pos: 0,
                cap: 0,
            }
        }
    }
    impl AsyncRead for ChildStdoutCompat {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            out: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            let me = &mut *self;
            if me.pos < me.cap {
                let n = out.len().min(me.cap - me.pos);
                out[..n].copy_from_slice(&me.buf[me.pos..me.pos + n]);
                me.pos += n;
                return Poll::Ready(Ok(n));
            }
            let mut rb = tokio::io::ReadBuf::new(out);
            match Pin::new(&mut me.inner).poll_read(cx, &mut rb) {
                Poll::Ready(Ok(())) => Poll::Ready(Ok(rb.filled().len())),
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            }
        }
    }
    impl AsyncBufRead for ChildStdoutCompat {
        fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
            let me = self.get_mut();
            if me.pos >= me.cap {
                let mut rb = tokio::io::ReadBuf::new(&mut me.buf);
                match Pin::new(&mut me.inner).poll_read(cx, &mut rb) {
                    Poll::Ready(Ok(())) => {
                        me.cap = rb.filled().len();
                        me.pos = 0;
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            }
            Poll::Ready(Ok(&me.buf[me.pos..me.cap]))
        }
        fn consume(self: Pin<&mut Self>, amt: usize) {
            let me = self.get_mut();
            me.pos = (me.pos + amt).min(me.cap);
        }
    }
}

#[derive(Clone, Debug)]
pub enum CallControllerEvent {
    Changed,
    Fallback { key: CallKey, reason: FailReason },
    AskSwitch { from: CallKey, to: CallKey },
    Toast(Arc<str>),
}

struct Running {
    lines: mpsc::UnboundedSender<Zeroizing<String>>,
    kill: Option<Box<dyn FnOnce() + Send>>,
    alive: bool,
    _tasks: Vec<Task<()>>,
}

type TitleFormatter = Box<dyn Fn(&gpui::App, &str) -> String>;

pub struct CallController {
    spawner: Box<dyn HelperSpawner>,
    state: CallState,
    settings: CallSettings,
    provider: Option<Arc<dyn ChatProvider>>,
    rt: Option<tokio::runtime::Handle>,
    proc: Option<Running>,
    fetch: Option<Task<()>>,
    load_timer: Option<Task<()>>,
    grace: Option<Task<()>>,
    /// Localizes the call window title from the provider's conversation name.
    title: Option<TitleFormatter>,
}

impl EventEmitter<CallControllerEvent> for CallController {}

impl CallController {
    pub fn new(spawner: Box<dyn HelperSpawner>, _cx: &mut Context<Self>) -> Self {
        Self {
            spawner,
            state: CallState::idle(),
            settings: CallSettings::default(),
            provider: None,
            rt: None,
            proc: None,
            fetch: None,
            load_timer: None,
            grace: None,
            title: None,
        }
    }

    /// Localizes the call window title. Without it the provider's conversation
    /// name is used as given; the window is the app's, so its catalogue keys
    /// stay in the app.
    pub fn with_title(mut self, title: impl Fn(&gpui::App, &str) -> String + 'static) -> Self {
        self.title = Some(Box::new(title));
        self
    }

    /// Runs provider futures on the shared Tokio runtime (the app); tests leave
    /// it unset and run them on GPUI's executor.
    pub fn with_runtime(mut self, rt: tokio::runtime::Handle) -> Self {
        self.rt = Some(rt);
        self
    }

    pub fn set_settings(&mut self, s: CallSettings) {
        self.settings = s;
    }

    pub fn state(&self) -> &CallState {
        &self.state
    }

    pub fn open(&mut self, key: CallKey, provider: Arc<dyn ChatProvider>, cx: &mut Context<Self>) {
        if self.state.phase == crate::session::CallPhase::Idle {
            self.provider = Some(provider);
        }
        self.feed(CallInput::Open(key), cx);
    }

    pub fn leave(&mut self, cx: &mut Context<Self>) {
        self.feed(CallInput::Leave, cx);
    }
    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.feed(CallInput::ToggleMute, cx);
    }
    pub fn show(&mut self, cx: &mut Context<Self>) {
        self.feed(CallInput::Show, cx);
    }
    pub fn huddle_ended(&mut self, key: CallKey, cx: &mut Context<Self>) {
        self.feed(CallInput::HuddleEnded(key), cx);
    }

    fn feed(&mut self, input: CallInput, cx: &mut Context<Self>) {
        let fx = step(&mut self.state, input, &self.settings);
        for e in fx {
            self.run(e, cx);
        }
        if self.state.phase == crate::session::CallPhase::Idle {
            self.teardown();
        }
        cx.emit(CallControllerEvent::Changed);
        cx.notify();
    }

    fn teardown(&mut self) {
        self.grace = None;
        self.load_timer = None;
        self.fetch = None;
        if let Some(mut p) = self.proc.take()
            && p.alive
            && let Some(k) = p.kill.take()
        {
            k();
        }
    }

    fn kill(&mut self) {
        if let Some(p) = self.proc.as_mut()
            && let Some(k) = p.kill.take()
        {
            k();
            p.alive = false;
        }
    }

    fn run(&mut self, e: CallEffect, cx: &mut Context<Self>) {
        match e {
            CallEffect::FetchSurface(key) => self.fetch_surface(key, cx),
            CallEffect::Spawn(surface) => self.spawn(surface, cx),
            CallEffect::Send(m) => self.send(&m, cx),
            CallEffect::StartLoadTimer => {
                self.load_timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(LOAD_TIMEOUT).await;
                    this.update(cx, |c, cx| c.feed(CallInput::LoadTimeout, cx))
                        .ok();
                }));
            }
            CallEffect::CancelLoadTimer => self.load_timer = None,
            CallEffect::Kill => self.kill(),
            CallEffect::Fallback { key, reason } => {
                cx.emit(CallControllerEvent::Fallback { key, reason });
            }
            CallEffect::AskSwitch { from, to } => {
                cx.emit(CallControllerEvent::AskSwitch { from, to });
            }
            CallEffect::LeaveProvider(key) => self.leave_provider(key, cx),
            CallEffect::Toast(t) => cx.emit(CallControllerEvent::Toast(t)),
        }
    }

    fn fetch_surface(&mut self, key: CallKey, cx: &mut Context<Self>) {
        let Some(p) = self.provider.clone() else {
            self.feed(CallInput::SurfaceFailed, cx);
            return;
        };
        let rt = self.rt.clone();
        let conv = key.conversation;
        // Replacing the task drops (cancels) a stale fetch.
        self.fetch = Some(cx.spawn(async move |this, cx| {
            let fut = async move { p.call_surface(&conv).await };
            let res = match rt {
                Some(rt) => rt
                    .spawn(fut)
                    .await
                    .map_err(|_| ())
                    .and_then(|r| r.map_err(|_| ())),
                None => fut.await.map_err(|_| ()),
            };
            this.update(cx, |c, cx| {
                c.fetch = None;
                match res {
                    Ok(mut s) => {
                        // The window title is the app's, so the app localizes it.
                        if let Some(f) = &c.title {
                            let name = s.title.to_string();
                            s.title = Arc::from(f(cx, &name));
                        }
                        c.feed(CallInput::SurfaceReady(s), cx)
                    }
                    Err(()) => c.feed(CallInput::SurfaceFailed, cx),
                }
            })
            .ok();
        }));
    }

    fn leave_provider(&mut self, key: CallKey, cx: &mut Context<Self>) {
        let Some(p) = self.provider.clone() else {
            return;
        };
        let conv = key.conversation;
        let rt = self.rt.clone();
        cx.spawn(async move |_, _| {
            let fut = async move { p.leave_huddle(&conv).await };
            let res = match rt {
                Some(rt) => rt.spawn(fut).await.ok(),
                None => Some(fut.await),
            };
            if let Some(Err(e)) = res {
                tracing::debug!(kind = ?std::mem::discriminant(&e), "leave_huddle failed");
            }
        })
        .detach();
    }

    fn spawn(&mut self, surface: domain::CallSurface, cx: &mut Context<Self>) {
        let HelperProcess {
            stdin,
            stdout,
            kill,
            exited,
        } = match self.spawner.spawn() {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(kind = ?e.kind(), "call helper failed to start");
                self.feed(CallInput::HelperStartFailed, cx);
                return;
            }
        };
        let (tx, mut rx) = mpsc::unbounded::<Zeroizing<String>>();
        let open = Zeroizing::new(encode_parent(&ParentMsg::Open(WireSurface::from(surface))));
        let _ = tx.unbounded_send(open);

        let writer = cx.background_spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = rx.next().await {
                if stdin.write_all(line.as_bytes()).await.is_err()
                    || stdin.write_all(b"\n").await.is_err()
                    || stdin.flush().await.is_err()
                {
                    break;
                }
            }
            // Dropping the sender closed the pipe: the helper sees EOF.
            let _ = stdin.close().await;
        });
        let reader = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let mut stdout = stdout;
            let mut line = String::new();
            loop {
                match read_bounded_line(&mut stdout, &mut line).await {
                    Ok(Some(true)) => {}
                    Ok(Some(false)) => {
                        tracing::debug!("ignored an oversized helper line");
                        continue;
                    }
                    Ok(None) | Err(_) => break,
                }
                match decode_helper(line.trim_end()) {
                    Ok(m) => {
                        if this
                            .update(cx, |c, cx| c.feed(CallInput::Helper(m), cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(e) => tracing::debug!(error = %e, "ignored a malformed helper line"),
                }
            }
        });
        let waiter = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            exited.await;
            this.update(cx, |c, cx| {
                if let Some(p) = c.proc.as_mut() {
                    p.alive = false;
                }
                c.feed(CallInput::HelperExited, cx);
            })
            .ok();
        });
        self.proc = Some(Running {
            lines: tx,
            kill: Some(kill),
            alive: true,
            _tasks: vec![writer, reader, waiter],
        });
    }

    fn send(&mut self, m: &ParentMsg, cx: &mut Context<Self>) {
        let Some(p) = self.proc.as_ref() else { return };
        let _ = p.lines.unbounded_send(Zeroizing::new(encode_parent(m)));
        if matches!(m, ParentMsg::Leave) {
            self.grace = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(LEAVE_GRACE).await;
                this.update(cx, |c, cx| {
                    c.kill();
                    c.feed(CallInput::HelperExited, cx);
                })
                .ok();
            }));
        }
    }
}

/// Reads one line into `line` (cleared first) without ever buffering more than
/// `MAX_LINE + 1` bytes of it. `Some(true)`: a line to decode; `Some(false)`:
/// the line was too long and has been skipped to its end; `None`: end of input.
async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    r: &mut R,
    line: &mut String,
) -> io::Result<Option<bool>> {
    use futures::io::AsyncReadExt;
    line.clear();
    if (&mut *r).take(MAX_LINE as u64 + 1).read_line(line).await? == 0 {
        return Ok(None);
    }
    if line.len() <= MAX_LINE || line.ends_with('\n') {
        return Ok(Some(true));
    }
    // Too long and not finished: drop it and skip the rest of the line.
    line.clear();
    let mut rest = String::new();
    loop {
        rest.clear();
        let n = (&mut *r)
            .take(MAX_LINE as u64 + 1)
            .read_line(&mut rest)
            .await?;
        if n == 0 || rest.ends_with('\n') {
            return Ok(Some(false));
        }
    }
}

impl Drop for CallController {
    fn drop(&mut self) {
        self.teardown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{executor::block_on, io::Cursor};

    #[test]
    fn a_long_line_is_never_buffered_whole() {
        let mut data = "a".repeat(MAX_LINE * 3).into_bytes();
        data.extend_from_slice(b"\n{\"type\":\"ok\"}\n");
        let mut r = Cursor::new(data);
        let mut line = String::new();
        assert_eq!(
            block_on(read_bounded_line(&mut r, &mut line)).unwrap(),
            Some(false)
        );
        assert!(line.len() <= MAX_LINE + 1);
        assert_eq!(
            block_on(read_bounded_line(&mut r, &mut line)).unwrap(),
            Some(true)
        );
        assert_eq!(line.trim_end(), "{\"type\":\"ok\"}");
        assert_eq!(
            block_on(read_bounded_line(&mut r, &mut line)).unwrap(),
            None
        );
    }

    #[test]
    fn a_line_of_exactly_max_len_is_kept() {
        let mut r = Cursor::new(format!("{}\n", "b".repeat(MAX_LINE - 1)).into_bytes());
        let mut line = String::new();
        assert_eq!(
            block_on(read_bounded_line(&mut r, &mut line)).unwrap(),
            Some(true)
        );
    }
}
