//! A fake call helper over in-memory pipes, plus the globals a call needs.
//! Shared by the call, perf and `CallManager` tests.

use std::io;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use calls::controller::{HelperProcess, HelperSpawner};
use calls::{HelperMsg, encode_helper};
use components::CallBarModel;
use domain::test_support::{FakeProvider, cid, conv, member, uid};
use domain::{
    CallSurface, Capabilities, ChatProvider, ConversationKind, Huddle, HuddleId, SecretText,
};
use futures::channel::{mpsc, oneshot};
use futures::io::AsyncWrite;
use futures::{FutureExt, TryStreamExt};
use gpui::{App, TestAppContext};
use parla::call_manager::{CallManager, CallManagerHandle};
use parla::notifications::{GpuiNotifier, Notifications};
use parla::ringer::{Ringer, RingtoneHandle, RingtonePlayer};
use settings::{CallSettings, ConfigPaths};

struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl AsyncWrite for SharedWriter {
    fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, b: &[u8]) -> Poll<io::Result<usize>> {
        self.0.lock().unwrap().extend_from_slice(b);
        Poll::Ready(Ok(b.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[derive(Default)]
struct Helper {
    written: Arc<Mutex<Vec<u8>>>,
    out: Mutex<Option<mpsc::UnboundedSender<io::Result<Vec<u8>>>>>,
    exit: Mutex<Option<oneshot::Sender<()>>>,
    kills: AtomicUsize,
    exits: AtomicUsize,
    spawns: AtomicUsize,
    /// Helpers spawned and not yet exited or killed.
    alive: AtomicUsize,
    /// The next spawn fails, as a missing helper binary.
    fail: Mutex<bool>,
}

/// The fake spawner: records every helper and lets the test drive it.
#[derive(Clone, Default)]
pub struct Fake(Arc<Helper>);

impl HelperSpawner for Fake {
    fn spawn(&self) -> io::Result<HelperProcess> {
        if std::mem::take(&mut *self.0.fail.lock().unwrap()) {
            return Err(io::Error::other("no such binary"));
        }
        self.0.spawns.fetch_add(1, Ordering::SeqCst);
        self.0.alive.fetch_add(1, Ordering::SeqCst);
        self.0.written.lock().unwrap().clear();
        let (otx, orx) = mpsc::unbounded();
        let (etx, erx) = oneshot::channel::<()>();
        *self.0.out.lock().unwrap() = Some(otx);
        *self.0.exit.lock().unwrap() = Some(etx);
        let h = self.0.clone();
        Ok(HelperProcess {
            stdin: Box::new(SharedWriter(self.0.written.clone())),
            stdout: Box::new(orx.into_async_read()),
            kill: Box::new(move || {
                h.kills.fetch_add(1, Ordering::SeqCst);
                if let Some(t) = h.exit.lock().unwrap().take() {
                    h.alive.fetch_sub(1, Ordering::SeqCst);
                    let _ = t.send(());
                }
            }),
            exited: erx.map(|_| ()).boxed(),
        })
    }
}

impl Fake {
    /// Makes the next spawn fail, as a missing helper binary.
    pub fn fail_next_spawn(&self) {
        *self.0.fail.lock().unwrap() = true;
    }

    fn say(&self, m: HelperMsg) {
        let mut line = encode_helper(&m).into_bytes();
        line.push(b'\n');
        let _ = self
            .0
            .out
            .lock()
            .unwrap()
            .as_ref()
            .expect("a helper was spawned")
            .unbounded_send(Ok(line));
    }

    /// The helper reported the window loaded: the call is live.
    pub fn live(&self) {
        self.say(HelperMsg::Ready);
        self.say(HelperMsg::Loaded {
            host: "app.example.test".into(),
            path: "/h".into(),
        });
    }

    /// The helper saw `Leave` and closed itself, as the real one does.
    pub fn exit(&self) {
        if let Some(t) = self.0.exit.lock().unwrap().take() {
            self.0.exits.fetch_add(1, Ordering::SeqCst);
            self.0.alive.fetch_sub(1, Ordering::SeqCst);
            let _ = t.send(());
        }
    }

    /// Lines the parent wrote to the helper's stdin.
    pub fn lines(&self) -> Vec<String> {
        String::from_utf8_lossy(&self.0.written.lock().unwrap())
            .lines()
            .map(str::to_owned)
            .collect()
    }

    pub fn spawns(&self) -> usize {
        self.0.spawns.load(Ordering::SeqCst)
    }

    pub fn kills(&self) -> usize {
        self.0.kills.load(Ordering::SeqCst)
    }

    pub fn exits(&self) -> usize {
        self.0.exits.load(Ordering::SeqCst)
    }

    pub fn alive(&self) -> usize {
        self.0.alive.load(Ordering::SeqCst)
    }
}

pub struct Silent;

impl RingtonePlayer for Silent {
    fn play(&self, _: bool) -> Option<RingtoneHandle> {
        None
    }
}

fn caps() -> Capabilities {
    Capabilities {
        huddles: true,
        call_surface: true,
        ..Default::default()
    }
}

fn surface() -> Result<CallSurface, domain::ProviderError> {
    Ok(CallSurface {
        url: Arc::from("https://app.example.test/h"),
        title: Arc::from("Huddle"),
        cookies: vec![domain::SurfaceCookie {
            name: Arc::from("d"),
            value: SecretText::new("secret-value"),
            domain: Arc::from(".example.test"),
            path: Arc::from("/"),
            secure: true,
            http_only: true,
        }],
        allowed_host_suffixes: Arc::from([Arc::from("example.test")]),
        signed_out_paths: Arc::from([Arc::from("/signin")]),
        user_agent: None,
    })
}

fn huddle() -> Huddle {
    Huddle {
        id: HuddleId(Arc::from("H-C1")),
        conversation: cid("C1"),
        participants: vec![uid("u2")].into(),
        started: 0,
        created_by: Some(uid("u2")),
        is_dm: false,
    }
}

/// Everything one call needs: the settings dir, the fake spawner and a provider
/// with a huddle in `C1` and a call surface.
pub struct Rig {
    _dir: tempfile::TempDir,
    pub fake: Fake,
    pub provider: Arc<FakeProvider>,
}

/// Installs the app globals a call touches and scripts a provider with a
/// huddle in `C1`. The huddle is provider-side only (no event is pushed), so
/// no huddle bar renders and its minute tick cannot be confused with a call
/// timer. No window either: the heap test wants the call path alone.
pub fn rig(cx: &mut TestAppContext) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    std::fs::write(
        &paths.settings,
        r#"{ "locale": "en", "calls": { "join_with": "window" } }"#,
    )
    .unwrap();
    let fake = Fake::default();
    let f = fake.clone();
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        cx.set_app_identity("com.justparla.parla", "Parla");
        Notifications::new(Rc::new(GpuiNotifier::new())).install(cx);
        Ringer::new(Rc::new(Silent), cx).install(cx);
        CallManager::install(Box::new(f), cx).update(cx, |m, _| m.set_window_supported(true));
    });
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")).with_capabilities(caps()));
    provider.set_call_surface(surface);
    provider.add_conversation(conv("C1", "general"));
    let mut dm = conv("D1", "dm");
    dm.kind = ConversationKind::Direct;
    provider.add_conversation(dm);
    provider.add_member(member("u1", "Me"));
    provider.add_member(member("u2", "Ada"));
    provider.set_huddles(vec![huddle()]);
    Rig {
        _dir: dir,
        fake,
        provider,
    }
}

/// Joins `C1`'s huddle through `CallManager` (the window rung).
pub fn join(rig: &Rig, cx: &mut App) {
    let ws = rig.provider.workspace().clone();
    let provider: Arc<dyn ChatProvider> = rig.provider.clone();
    let calls = CallSettings::default();
    let manager = cx.global::<CallManagerHandle>().0.clone();
    manager.update(cx, |m, cx| {
        m.join(&ws, cid("C1"), provider, &calls, cx);
    });
}

pub fn leave(cx: &mut App) {
    CallManager::leave(cx);
}

pub fn in_call(cx: &App) -> bool {
    cx.global::<CallManagerHandle>().0.read(cx).in_call(cx)
}

pub fn bar_model(cx: &App) -> Option<CallBarModel> {
    cx.global::<CallManagerHandle>().0.read(cx).bar_model(cx)
}

pub fn ringer_timer_armed(cx: &App) -> bool {
    cx.global::<Ringer>().timer_armed()
}
