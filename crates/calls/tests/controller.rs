//! `CallController` against a scripted fake helper (in-memory pipes) and
//! `FakeProvider`.

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use calls::controller::{
    CallController, CallControllerEvent, ExeSpawner, HelperProcess, HelperSpawner,
};
use calls::{CallKey, CallPhase, FailReason, HelperMsg, ParentMsg, decode_parent, encode_helper};
use domain::test_support::{Call, FakeProvider, uid};
use domain::{CallSurface, Capabilities, ChatProvider, ConversationId, SecretText};
use futures::channel::{mpsc, oneshot};
use futures::io::AsyncWrite;
use futures::{FutureExt, TryStreamExt};
use gpui::{AppContext as _, Entity, TestAppContext};
use settings::WorkspaceKey;

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
    spawns: AtomicUsize,
    fail: Mutex<bool>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Helper>);

impl HelperSpawner for Fake {
    fn spawn(&self) -> io::Result<HelperProcess> {
        if *self.0.fail.lock().unwrap() {
            return Err(io::Error::other("no such binary"));
        }
        self.0.spawns.fetch_add(1, Ordering::SeqCst);
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
                    let _ = t.send(());
                }
            }),
            exited: erx.map(|_| ()).boxed(),
        })
    }
}

impl Fake {
    fn say(&self, m: HelperMsg) {
        let mut line = encode_helper(&m).into_bytes();
        line.push(b'\n');
        let _ = self
            .0
            .out
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .unbounded_send(Ok(line));
    }
    fn say_raw(&self, s: &str) {
        let _ = self
            .0
            .out
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .unbounded_send(Ok(s.as_bytes().to_vec()));
    }
    fn exit(&self) {
        if let Some(t) = self.0.exit.lock().unwrap().take() {
            let _ = t.send(());
        }
    }
    fn lines(&self) -> Vec<String> {
        String::from_utf8_lossy(&self.0.written.lock().unwrap())
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn kills(&self) -> usize {
        self.0.kills.load(Ordering::SeqCst)
    }
}

fn key() -> CallKey {
    CallKey {
        workspace: WorkspaceKey::new("fake", "w1"),
        conversation: ConversationId(Arc::from("C1")),
    }
}

type Rig = (
    Fake,
    Arc<FakeProvider>,
    Entity<CallController>,
    Arc<Mutex<Vec<String>>>,
);

fn setup(cx: &mut TestAppContext) -> Rig {
    setup_with_title(cx, "Huddle")
}

fn setup_with_title(cx: &mut TestAppContext, title: &str) -> Rig {
    let title = Arc::<str>::from(title);
    let fake = Fake::default();
    let provider = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
            call_surface: true,
            huddles: true,
            ..Default::default()
        }),
    );
    provider.set_call_surface(move || {
        Ok(CallSurface {
            url: Arc::from("https://app.example.test/h"),
            title: title.clone(),
            cookies: vec![domain::SurfaceCookie {
                name: Arc::from("d"),
                value: SecretText::new("sekrit-value"),
                domain: Arc::from(".example.test"),
                path: Arc::from("/"),
                secure: true,
                http_only: true,
            }],
            allowed_host_suffixes: Arc::from([Arc::from("example.test")]),
            signed_out_paths: Arc::from([Arc::from("/signin")]),
            user_agent: None,
        })
    });
    // The app supplies the real one; this is the same shape of title.
    let ctl = cx.new(|cx| {
        CallController::new(Box::new(fake.clone()), cx).with_title(|_, name| {
            if name.is_empty() {
                "Huddle".to_string()
            } else {
                format!("Huddle in {name}")
            }
        })
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    cx.update(|cx| {
        cx.subscribe(&ctl, move |_, e: &CallControllerEvent, _| {
            let s = match e {
                CallControllerEvent::Changed => return,
                CallControllerEvent::Fallback { reason, .. } => format!("fallback:{reason:?}"),
                CallControllerEvent::AskSwitch { .. } => "ask".to_owned(),
                CallControllerEvent::Toast(t) => format!("toast:{t}"),
            };
            ev.lock().unwrap().push(s);
        })
        .detach();
    });
    (fake, provider, ctl, events)
}

fn open(ctl: &Entity<CallController>, p: &Arc<FakeProvider>, cx: &mut TestAppContext) {
    let p: Arc<dyn ChatProvider> = p.clone();
    ctl.update(cx, |c, cx| c.open(key(), p, cx));
    cx.run_until_parked();
}

fn loaded() -> HelperMsg {
    HelperMsg::Loaded {
        host: "app.example.test".into(),
        path: "/h".into(),
    }
}

fn phase(ctl: &Entity<CallController>, cx: &TestAppContext) -> CallPhase {
    ctl.read_with(cx, |c, _| c.state().phase)
}

#[gpui::test]
async fn non_empty_name_wraps_title_in_huddle_in(cx: &mut TestAppContext) {
    let (fake, p, ctl, _ev) = setup_with_title(cx, "#general");
    open(&ctl, &p, cx);
    let lines = fake.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let Ok(ParentMsg::Open(s)) = decode_parent(&lines[0]) else {
        panic!("first line is not Open");
    };
    assert_eq!(&*s.title, "Huddle in #general");
}

#[gpui::test]
async fn empty_name_uses_huddle_title(cx: &mut TestAppContext) {
    let (fake, p, ctl, _ev) = setup_with_title(cx, "");
    open(&ctl, &p, cx);
    let lines = fake.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let Ok(ParentMsg::Open(s)) = decode_parent(&lines[0]) else {
        panic!("first line is not Open");
    };
    assert_eq!(&*s.title, "Huddle");
}

#[gpui::test]
async fn open_writes_one_open_line_with_cookie_only_on_stdin(cx: &mut TestAppContext) {
    let (fake, p, ctl, _ev) = setup(cx);
    open(&ctl, &p, cx);
    let lines = fake.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let Ok(ParentMsg::Open(s)) = decode_parent(&lines[0]) else {
        panic!("first line is not Open");
    };
    assert_eq!(&*s.url, "https://app.example.test/h");
    assert_eq!(s.cookies[0].value.expose(), "sekrit-value");
    assert!(!format!("{:?}", ctl.read_with(cx, |c, _| c.state().clone())).contains("sekrit"));
}

#[test]
fn spawn_carries_no_secret_in_args_or_env() {
    let cmd = ExeSpawner::default().command().expect("current exe");
    let args: Vec<_> = cmd.get_args().collect();
    assert_eq!(args, ["call-window"]);
    assert_eq!(
        cmd.get_envs().count(),
        0,
        "nothing added to the environment"
    );
}

#[gpui::test]
async fn loaded_goes_live_and_cancels_timer(cx: &mut TestAppContext) {
    let (fake, p, ctl, ev) = setup(cx);
    open(&ctl, &p, cx);
    fake.say(HelperMsg::Ready);
    fake.say(loaded());
    cx.run_until_parked();
    assert_eq!(phase(&ctl, cx), CallPhase::Live);
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert!(!ctl.read_with(cx, |c, _| c.state().load_slow));
    assert!(ev.lock().unwrap().is_empty());
}

#[gpui::test]
async fn load_timeout_marks_slow(cx: &mut TestAppContext) {
    let (_fake, p, ctl, _ev) = setup(cx);
    open(&ctl, &p, cx);
    cx.executor().advance_clock(Duration::from_secs(29));
    cx.run_until_parked();
    assert!(!ctl.read_with(cx, |c, _| c.state().load_slow));
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert!(ctl.read_with(cx, |c, _| c.state().load_slow));
}

#[gpui::test]
async fn leave_kills_helper_within_2s(cx: &mut TestAppContext) {
    let (fake, p, ctl, _ev) = setup(cx);
    open(&ctl, &p, cx);
    fake.say(loaded());
    cx.run_until_parked();
    ctl.update(cx, |c, cx| c.leave(cx));
    cx.run_until_parked();
    assert!(
        fake.lines().iter().any(|l| l.contains("\"leave\"")),
        "{:?}",
        fake.lines().len()
    );
    assert_eq!(fake.kills(), 0, "the helper gets its grace first");
    cx.executor().advance_clock(Duration::from_millis(1900));
    cx.run_until_parked();
    assert_eq!(fake.kills(), 0);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(fake.kills(), 1);
    assert_eq!(phase(&ctl, cx), CallPhase::Idle);
    assert!(p.calls().contains(&Call::LeaveHuddle(key().conversation)));
}

#[gpui::test]
async fn helper_crash_before_live_emits_fallback(cx: &mut TestAppContext) {
    let (fake, p, ctl, ev) = setup(cx);
    open(&ctl, &p, cx);
    fake.exit();
    cx.run_until_parked();
    assert_eq!(
        *ev.lock().unwrap(),
        vec![format!("fallback:{:?}", FailReason::Crashed)]
    );
    assert_eq!(phase(&ctl, cx), CallPhase::Idle);
}

#[gpui::test]
async fn window_closed_calls_leave_huddle(cx: &mut TestAppContext) {
    let (fake, p, ctl, _ev) = setup(cx);
    open(&ctl, &p, cx);
    fake.say(loaded());
    fake.say(HelperMsg::WindowClosed);
    cx.run_until_parked();
    assert_eq!(phase(&ctl, cx), CallPhase::Idle);
    assert!(p.calls().contains(&Call::LeaveHuddle(key().conversation)));
    assert_eq!(fake.kills(), 1);
}

#[gpui::test]
async fn spawn_failure_emits_fallback(cx: &mut TestAppContext) {
    let (fake, p, ctl, ev) = setup(cx);
    *fake.0.fail.lock().unwrap() = true;
    open(&ctl, &p, cx);
    assert_eq!(
        *ev.lock().unwrap(),
        vec![format!("fallback:{:?}", FailReason::HelperStart)]
    );
    assert_eq!(phase(&ctl, cx), CallPhase::Idle);
}

#[gpui::test]
async fn dropping_controller_kills_helper(cx: &mut TestAppContext) {
    let (fake, p, ctl, _ev) = setup(cx);
    open(&ctl, &p, cx);
    fake.say(loaded());
    cx.run_until_parked();
    cx.update(move |_| drop(ctl));
    cx.run_until_parked();
    assert_eq!(fake.kills(), 1);
}

#[gpui::test]
async fn malformed_helper_line_is_ignored(cx: &mut TestAppContext) {
    let (fake, p, ctl, ev) = setup(cx);
    open(&ctl, &p, cx);
    fake.say_raw("not json\n{\"type\":\"mystery\"}\n");
    fake.say(loaded());
    cx.run_until_parked();
    assert_eq!(phase(&ctl, cx), CallPhase::Live);
    assert!(ev.lock().unwrap().is_empty());
}

#[gpui::test]
async fn second_open_for_other_conversation_asks(cx: &mut TestAppContext) {
    let (fake, p, ctl, ev) = setup(cx);
    open(&ctl, &p, cx);
    fake.say(loaded());
    cx.run_until_parked();
    let other = CallKey {
        conversation: ConversationId(Arc::from("C2")),
        ..key()
    };
    let pr: Arc<dyn ChatProvider> = p.clone();
    ctl.update(cx, |c, cx| c.open(other, pr, cx));
    cx.run_until_parked();
    assert_eq!(*ev.lock().unwrap(), vec!["ask".to_owned()]);
}
