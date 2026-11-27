mod support {
    pub mod fake_server;
}

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use slack_api::{ApiError, SessionCookie, XoxcToken};
use slack_realtime::conn::{ConnConfig, ConnEvent, UrlSource, spawn};
use slack_realtime::event::{Outgoing, SlackEvent};
use slack_realtime::url::{WS_BASE, ws_url_on};
use support::fake_server::{FakeServer, Script, Step};
use tokio::sync::mpsc;
use url::Url;

const COOKIE: &str = "xoxd-synthetic%2Fcookie";
const TOKEN: &str = "xoxc-synthetic-token";

/// Serves queued results, then repeats `fallback`; counts fetches.
struct FakeUrls {
    queue: Mutex<VecDeque<Result<Url, ApiError>>>,
    fallback: Result<Url, ApiError>,
    fetches: AtomicUsize,
    times: Mutex<Vec<tokio::time::Instant>>,
}

impl FakeUrls {
    fn new(fallback: Result<Url, ApiError>) -> Arc<FakeUrls> {
        Arc::new(FakeUrls {
            queue: Mutex::new(VecDeque::new()),
            fallback,
            fetches: AtomicUsize::new(0),
            times: Mutex::new(Vec::new()),
        })
    }
    fn count(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }
}

impl UrlSource for FakeUrls {
    fn fetch(&self) -> BoxFuture<'_, Result<Url, ApiError>> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        self.times.lock().unwrap().push(tokio::time::Instant::now());
        let r = self
            .queue
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| self.fallback.clone());
        Box::pin(async move { r })
    }
}

fn cfg() -> ConnConfig {
    let mut c = ConnConfig::new(SessionCookie::parse(COOKIE).unwrap());
    c.user_agent = Some("UA/1".into());
    c.origin = Some("https://app.example.test".into());
    c.retry_floor = Duration::ZERO;
    c
}

fn hello() -> Step {
    Step::Send(r#"{"type":"hello"}"#.into())
}

async fn next(rx: &mut mpsc::Receiver<ConnEvent>) -> ConnEvent {
    tokio::time::timeout(Duration::from_secs(600), rx.recv())
        .await
        .expect("event in time")
        .expect("channel open")
}

#[test]
fn ws_url_matches_web_client_params() {
    let token = XoxcToken::parse(TOKEN).unwrap();
    let url = ws_url_on(
        &Url::parse(WS_BASE).unwrap(),
        "T0TEAM",
        &token,
        "1794521867",
    );
    assert_eq!(url.scheme(), "wss");
    assert_eq!(url.host_str(), Some("wss-primary.slack.com"));
    let q: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    let names: Vec<&str> = q.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        names,
        [
            "token",
            "sync_desync",
            "slack_client",
            "start_args",
            "no_query_on_subscribe",
            "flannel",
            "lazy_channels",
            "gateway_server",
            "batch_presence_aware"
        ]
    );
    let get = |k: &str| q.iter().find(|(n, _)| n == k).unwrap().1.clone();
    assert_eq!(get("token"), TOKEN);
    assert_eq!(get("sync_desync"), "1");
    assert_eq!(get("slack_client"), "desktop");
    assert_eq!(get("flannel"), "3");
    assert_eq!(get("gateway_server"), "T0TEAM-1");
    assert_eq!(
        get("start_args"),
        "?agent=client&org_wide_aware=true&agent_version=1794521867&eac_cache_ts=true\
&cache_ts=0&name_tagging=true&only_self_subteams=true&connect_only=true&ms_latest=true"
    );
}

#[tokio::test]
async fn handshake_sends_cookie_origin_ua() {
    let server = FakeServer::start(vec![Script {
        steps: vec![hello()],
        ..Script::default()
    }])
    .await;
    let urls = FakeUrls::new(Ok(server.url()));
    let mut c = cfg();
    c.extra_headers.push(("x-extra".into(), "1".into()));
    let (h, mut rx) = spawn(c, urls, &tokio::runtime::Handle::current());
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Connected { reconnect: false }
    ));
    let hs = &server.handshakes()[0];
    assert_eq!(hs.headers["cookie"], format!("d={COOKIE}"));
    assert_eq!(hs.headers["origin"], "https://app.example.test");
    assert_eq!(hs.headers["user-agent"], "UA/1");
    assert_eq!(hs.headers["x-extra"], "1");
    assert!(hs.query.is_empty());
    h.shutdown();
}

#[tokio::test]
async fn hello_yields_connected_then_events() {
    let server = FakeServer::start(vec![Script {
        steps: vec![
            hello(),
            Step::Send(r#"{"type":"message","channel":"C1","ts":"1.2","text":"hi"}"#.into()),
        ],
        ..Script::default()
    }])
    .await;
    let urls = FakeUrls::new(Ok(server.url()));
    let (h, mut rx) = spawn(cfg(), urls, &tokio::runtime::Handle::current());
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Connected { reconnect: false }
    ));
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Event(SlackEvent::Hello)
    ));
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Event(SlackEvent::Message { .. })
    ));
    h.shutdown();
}

#[tokio::test]
async fn first_non_hello_frame_also_means_connected() {
    let server = FakeServer::start(vec![Script {
        steps: vec![Step::Send(
            r#"{"type":"presence_change","presence":"away","users":["U1"]}"#.into(),
        )],
        ..Script::default()
    }])
    .await;
    let urls = FakeUrls::new(Ok(server.url()));
    let (h, mut rx) = spawn(cfg(), urls, &tokio::runtime::Handle::current());
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Connected { reconnect: false }
    ));
    h.shutdown();
}

#[tokio::test]
async fn drop_reconnects_with_fresh_url() {
    let server = FakeServer::start(vec![
        Script {
            steps: vec![hello(), Step::Drop],
            ..Script::default()
        },
        Script {
            steps: vec![hello()],
            ..Script::default()
        },
    ])
    .await;
    let urls = FakeUrls::new(Ok(server.url()));
    let mut c = cfg();
    c.backoff_min = Duration::from_millis(10);
    let (h, mut rx) = spawn(c, urls.clone(), &tokio::runtime::Handle::current());
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Connected { reconnect: false }
    ));
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Event(SlackEvent::Hello)
    ));
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Disconnected { .. }
    ));
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Connected { reconnect: true }
    ));
    assert_eq!(urls.count(), 2);
    h.shutdown();
}

#[tokio::test(start_paused = true)]
async fn backoff_grows_with_jitter_and_caps() {
    let urls = FakeUrls::new(Err(ApiError::Transport("down".into())));
    let (h, mut rx) = spawn(cfg(), urls.clone(), &tokio::runtime::Handle::current());
    // 9 failures: delays ~1,2,4,8,16,32,60,60
    for _ in 0..9 {
        assert!(matches!(
            next(&mut rx).await,
            ConnEvent::Disconnected { .. }
        ));
    }
    let times = urls.times.lock().unwrap().clone();
    let gaps: Vec<f64> = times
        .windows(2)
        .map(|w| (w[1] - w[0]).as_secs_f64())
        .collect();
    let base = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 60.0, 60.0];
    for (g, b) in gaps.iter().zip(base) {
        // backon jitter adds 0..=delay on top of the exponential delay, then caps at max
        assert!(
            *g >= b - 0.01 && *g <= (2.0 * b).min(60.0) + 0.01,
            "gap {g} base {b}"
        );
    }
    h.shutdown();
}

#[tokio::test]
async fn auth_error_stops_reconnecting() {
    let urls = FakeUrls::new(Err(ApiError::InvalidAuth));
    let (h, mut rx) = spawn(cfg(), urls.clone(), &tokio::runtime::Handle::current());
    assert!(matches!(next(&mut rx).await, ConnEvent::AuthExpired));
    assert!(rx.recv().await.is_none(), "task ends after AuthExpired");
    assert_eq!(urls.count(), 1);
    drop(h);
}

#[tokio::test]
async fn session_ended_close_means_auth_expired() {
    let server = FakeServer::start(vec![Script {
        steps: vec![hello(), Step::Close(1008, "invalid_auth")],
        ..Script::default()
    }])
    .await;
    let urls = FakeUrls::new(Ok(server.url()));
    let (h, mut rx) = spawn(cfg(), urls.clone(), &tokio::runtime::Handle::current());
    loop {
        match next(&mut rx).await {
            ConnEvent::AuthExpired => break,
            ConnEvent::Disconnected { .. } => panic!("must not reconnect"),
            _ => {}
        }
    }
    assert!(rx.recv().await.is_none());
    assert_eq!(urls.count(), 1);
    drop(h);
}

// The two tests below talk to the loopback fake server, so they run on real time
// with short configured durations: tokio's paused clock auto-advances whenever the
// runtime idles on real socket I/O, which fired timers before the server answered.

#[tokio::test]
async fn ping_only_after_silence() {
    let server = FakeServer::start(vec![Script {
        steps: vec![hello()],
        reply_pong: true,
    }])
    .await;
    let urls = FakeUrls::new(Ok(server.url()));
    let mut c = cfg();
    c.silence_before_ping = Duration::from_millis(300);
    let started = std::time::Instant::now();
    let (h, mut rx) = spawn(c, urls, &tokio::runtime::Handle::current());
    assert!(matches!(next(&mut rx).await, ConnEvent::Connected { .. }));
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Event(SlackEvent::Hello)
    ));
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Event(SlackEvent::Pong { .. })
    ));
    // Load can only delay the ping, never advance it: a lower bound is deterministic.
    assert!(
        started.elapsed() >= Duration::from_millis(300),
        "no ping before the silence window"
    );
    let got = server.wait_received(1).await;
    assert!(got[0].contains(r#""type":"ping""#));
    h.shutdown();
}

#[tokio::test]
async fn keepalives_scale_with_silence_not_workspaces() {
    // wake.idle: three silent sockets, at most one ping per socket per silence
    // window, whatever the number of sockets. Short durations, as above.
    const WINDOW: Duration = Duration::from_millis(200);
    const RUN: Duration = Duration::from_millis(1_100);
    let mut servers = Vec::new();
    let mut conns = Vec::new();
    for _ in 0..3 {
        let server = FakeServer::start(vec![Script {
            steps: vec![hello()],
            reply_pong: true,
        }])
        .await;
        let mut c = cfg();
        c.silence_before_ping = WINDOW;
        let (h, rx) = spawn(
            c,
            FakeUrls::new(Ok(server.url())),
            &tokio::runtime::Handle::current(),
        );
        servers.push(server);
        conns.push((h, rx));
    }
    let started = std::time::Instant::now();
    // Drain the events so the connections never block on a full channel.
    let drains: Vec<_> = conns
        .iter_mut()
        .map(|(_, rx)| async move {
            let _ = tokio::time::timeout(RUN, async { while rx.recv().await.is_some() {} }).await;
        })
        .collect();
    futures::future::join_all(drains).await;
    let windows = (started.elapsed().as_millis() / WINDOW.as_millis()) as usize;
    for (i, server) in servers.iter().enumerate() {
        let pings = server
            .received()
            .iter()
            .filter(|f| f.contains(r#""type":"ping""#))
            .count();
        assert!(pings >= 1, "socket {i} pinged after silence");
        assert!(
            pings <= windows,
            "socket {i}: {pings} pings in {windows} silence windows"
        );
    }
    for (h, _) in conns {
        h.shutdown();
    }
}

#[tokio::test]
async fn missing_pong_reconnects() {
    let server = FakeServer::start(vec![
        Script {
            steps: vec![hello()],
            reply_pong: false,
        },
        Script {
            steps: vec![hello()],
            reply_pong: true,
        },
    ])
    .await;
    let urls = FakeUrls::new(Ok(server.url()));
    let mut c = cfg();
    c.silence_before_ping = Duration::from_millis(50);
    c.pong_timeout = Duration::from_millis(50);
    c.backoff_min = Duration::from_millis(10);
    let (h, mut rx) = spawn(c, urls.clone(), &tokio::runtime::Handle::current());
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Connected { reconnect: false }
    ));
    loop {
        match next(&mut rx).await {
            ConnEvent::Disconnected { reason } => {
                assert!(reason.contains("pong"), "{reason}");
                break;
            }
            ConnEvent::Event(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(matches!(
        next(&mut rx).await,
        ConnEvent::Connected { reconnect: true }
    ));
    assert_eq!(urls.count(), 2);
    h.shutdown();
}

#[tokio::test]
async fn send_writes_frames_and_uses_shared_ids() {
    let server = FakeServer::start(vec![Script {
        steps: vec![hello()],
        ..Script::default()
    }])
    .await;
    let urls = FakeUrls::new(Ok(server.url()));
    let (h, mut rx) = spawn(cfg(), urls, &tokio::runtime::Handle::current());
    assert!(matches!(next(&mut rx).await, ConnEvent::Connected { .. }));
    let id = h.next_id();
    h.send(Outgoing::Typing {
        id,
        channel: "C1".into(),
        thread_ts: None,
    });
    let got = server.wait_received(1).await;
    assert!(got[0].contains(r#""type":"user_typing""#));
    assert!(h.next_id() > id);
    h.shutdown();
}

#[test]
fn debug_output_is_redacted() {
    let c = cfg();
    let s = format!("{c:?}");
    assert!(!s.contains("synthetic"), "{s}");
    let urls = FakeUrls::new(Ok(Url::parse("ws://h/?token=xoxc-secret").unwrap()));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (h, _rx) = spawn(cfg(), urls, rt.handle());
    assert!(!format!("{h:?}").contains("secret"));
}

/// Answers every handshake with `401`, like a stale `reconnect_url`.
async fn reject_server() -> Url {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n")
                .await;
        }
    });
    Url::parse(&format!("ws://{addr}/")).unwrap()
}

#[tokio::test]
async fn stale_reconnect_url_falls_back_to_a_fresh_url() {
    let stale = reject_server().await;
    let server = FakeServer::start(vec![
        Script {
            steps: vec![
                hello(),
                Step::Send(format!(r#"{{"type":"reconnect_url","url":"{stale}"}}"#)),
                Step::Sleep(Duration::from_millis(50)),
                Step::Drop,
            ],
            ..Script::default()
        },
        Script {
            steps: vec![hello()],
            ..Script::default()
        },
    ])
    .await;
    let urls = FakeUrls::new(Ok(server.url()));
    let mut c = cfg();
    c.backoff_min = Duration::from_millis(10);
    c.backoff_max = Duration::from_millis(20);
    let (h, mut rx) = spawn(c, urls.clone(), &tokio::runtime::Handle::current());
    let mut connected = 0;
    loop {
        match next(&mut rx).await {
            ConnEvent::AuthExpired => panic!("a stale reconnect url expired the session"),
            ConnEvent::Connected { .. } => {
                connected += 1;
                if connected == 2 {
                    break;
                }
            }
            _ => {}
        }
    }
    assert_eq!(urls.count(), 2, "the second fetch replaced the stale url");
    h.shutdown();
}

fn reconnect_url_step(to: &Url) -> Step {
    Step::Send(format!(r#"{{"type":"reconnect_url","url":"{to}"}}"#))
}

/// Fresh server `a` hands out a resume URL to `b`, which hands out one to `c`.
/// Returns (urls, connected count, fetch count after the connection on `b` ended).
async fn resume_chain(b_steps: impl FnOnce(&Url) -> Vec<Step>) -> (usize, String) {
    let c = FakeServer::start(vec![Script {
        steps: vec![hello()],
        reply_pong: true,
    }])
    .await;
    let b = FakeServer::start(vec![Script {
        steps: b_steps(&c.url()),
        reply_pong: false,
    }])
    .await;
    let a = FakeServer::start(vec![
        Script {
            steps: vec![
                hello(),
                reconnect_url_step(&b.url()),
                Step::Sleep(Duration::from_millis(20)),
                Step::Drop,
            ],
            reply_pong: true,
        },
        Script {
            steps: vec![hello()],
            reply_pong: true,
        },
    ])
    .await;
    let urls = FakeUrls::new(Ok(a.url()));
    let mut cf = cfg();
    cf.silence_before_ping = Duration::from_millis(50);
    cf.pong_timeout = Duration::from_millis(50);
    cf.backoff_min = Duration::from_millis(10);
    cf.backoff_max = Duration::from_millis(20);
    let (h, mut rx) = spawn(cf, urls.clone(), &tokio::runtime::Handle::current());
    let mut connected = 0;
    let mut resumed_reason = String::new();
    while connected < 3 {
        match next(&mut rx).await {
            ConnEvent::Connected { .. } => connected += 1,
            ConnEvent::Disconnected { reason } if reason.contains("(resume") => {
                resumed_reason = reason;
            }
            _ => {}
        }
    }
    h.shutdown();
    (urls.count(), resumed_reason)
}

#[tokio::test]
async fn silent_resumed_connection_reconnects_fresh() {
    // `b` says hello, hands out a resume URL, then goes silent and ignores pings.
    let (fetches, reason) = resume_chain(|c| vec![hello(), reconnect_url_step(c)]).await;
    assert!(reason.contains("pong timeout"), "{reason}");
    assert_eq!(fetches, 2, "the dead session's resume url was reused");
}

#[tokio::test]
async fn healthy_resumed_connection_still_resumes_after_a_drop() {
    let (fetches, _) = resume_chain(|c| {
        vec![
            hello(),
            reconnect_url_step(c),
            Step::Send(r#"{"type":"user_typing","channel":"C1","user":"U1"}"#.into()),
            Step::Sleep(Duration::from_millis(20)),
            Step::Drop,
        ]
    })
    .await;
    assert_eq!(fetches, 1, "a healthy session's resume url was dropped");
}

#[tokio::test]
async fn hello_then_hangup_keeps_backing_off() {
    let script = || Script {
        steps: vec![hello(), Step::Drop],
        ..Script::default()
    };
    let server = FakeServer::start((0..6).map(|_| script()).collect()).await;
    let urls = FakeUrls::new(Ok(server.url()));
    let mut c = cfg();
    c.backoff_min = Duration::from_millis(100);
    c.backoff_max = Duration::from_secs(5);
    let (h, mut rx) = spawn(c, urls.clone(), &tokio::runtime::Handle::current());
    let mut dropped = 0;
    while dropped < 4 {
        if matches!(next(&mut rx).await, ConnEvent::Disconnected { .. }) {
            dropped += 1;
        }
    }
    let t = urls.times.lock().unwrap().clone();
    // Delays double (100, 200, 400 ms plus jitter); a reset on every hello would keep
    // each gap near the 100 to 200 ms of the first one.
    let third = (t[3] - t[2]).as_millis();
    assert!(third >= 380, "third gap {third} ms did not back off");
    h.shutdown();
}

#[tokio::test]
async fn retry_now_cuts_the_backoff_wait_short() {
    let script = || Script {
        steps: vec![hello(), Step::Drop],
        ..Script::default()
    };
    let server = FakeServer::start((0..3).map(|_| script()).collect()).await;
    let urls = FakeUrls::new(Ok(server.url()));
    let mut c = cfg();
    c.backoff_min = Duration::from_secs(30);
    c.backoff_max = Duration::from_secs(30);
    let (h, mut rx) = spawn(c, urls.clone(), &tokio::runtime::Handle::current());
    loop {
        if matches!(next(&mut rx).await, ConnEvent::Disconnected { .. }) {
            break;
        }
    }
    // Without the retry the next attempt is 30 s away.
    h.retry_now();
    let started = std::time::Instant::now();
    loop {
        if matches!(next(&mut rx).await, ConnEvent::Connected { .. }) {
            break;
        }
    }
    assert!(started.elapsed() < Duration::from_secs(5));
    h.shutdown();
}

/// Collects the next `Disconnected` reason, skipping other events.
async fn next_reason(rx: &mut mpsc::Receiver<ConnEvent>) -> String {
    loop {
        if let ConnEvent::Disconnected { reason } = next(rx).await {
            return reason;
        }
    }
}

#[tokio::test]
async fn disconnect_reason_names_the_cause_the_url_kind_and_uptime() {
    let server = FakeServer::start(vec![
        Script {
            steps: vec![hello(), Step::Send(r#"{"type":"goodbye"}"#.into())],
            ..Script::default()
        },
        Script {
            steps: vec![hello(), Step::Close(1001, "going away")],
            ..Script::default()
        },
    ])
    .await;
    let urls = FakeUrls::new(Ok(server.url()));
    let mut c = cfg();
    c.backoff_min = Duration::from_millis(10);
    let (h, mut rx) = spawn(c, urls.clone(), &tokio::runtime::Handle::current());
    let first = next_reason(&mut rx).await;
    assert!(first.starts_with("server goodbye"), "{first}");
    assert!(
        first.contains("fresh") && first.contains("up 0s"),
        "{first}"
    );
    let second = next_reason(&mut rx).await;
    assert!(second.starts_with("closed by server (1001)"), "{second}");
    for r in [&first, &second] {
        assert!(!r.contains("ws://") && !r.contains("token"), "{r}");
    }
    h.shutdown();
}

#[tokio::test]
async fn a_resumed_connection_says_so() {
    let server = FakeServer::start(Vec::new()).await;
    let resume = format!(r#"{{"type":"reconnect_url","url":"{}"}}"#, server.url());
    server.push_script(Script {
        steps: vec![hello(), Step::Send(resume), Step::Drop],
        ..Script::default()
    });
    server.push_script(Script {
        steps: vec![hello(), Step::Drop],
        ..Script::default()
    });
    let urls = FakeUrls::new(Ok(server.url()));
    let mut c = cfg();
    c.backoff_min = Duration::from_millis(10);
    let (h, mut rx) = spawn(c, urls.clone(), &tokio::runtime::Handle::current());
    let first = next_reason(&mut rx).await;
    assert!(first.contains("fresh"), "{first}");
    let second = next_reason(&mut rx).await;
    assert!(second.contains("resume"), "{second}");
    // The resume URL replaced the fetch for the second attempt.
    assert_eq!(urls.count(), 1);
    h.shutdown();
}

/// A caller that asks for a retry after every failure (queued sends) must not
/// turn a hello-then-hang-up server into a tight reconnect loop.
#[tokio::test]
async fn retry_now_keeps_a_minimum_gap_between_attempts() {
    let script = || Script {
        steps: vec![hello(), Step::Drop],
        ..Script::default()
    };
    let server = FakeServer::start((0..4).map(|_| script()).collect()).await;
    let urls = FakeUrls::new(Ok(server.url()));
    let mut c = cfg();
    c.backoff_min = Duration::from_secs(30);
    c.backoff_max = Duration::from_secs(30);
    c.retry_floor = Duration::from_millis(400);
    let (h, mut rx) = spawn(c, urls.clone(), &tokio::runtime::Handle::current());
    for _ in 0..3 {
        next_reason(&mut rx).await;
        h.retry_now();
    }
    let t = urls.times.lock().unwrap().clone();
    assert!(t.len() >= 3, "{} attempts", t.len());
    for w in t.windows(2) {
        let gap = (w[1] - w[0]).as_millis();
        assert!(gap >= 390, "attempts {gap} ms apart");
    }
    h.shutdown();
}

#[tokio::test(start_paused = true)]
async fn a_failing_url_fetch_names_its_kind_and_keeps_backing_off() {
    let urls = FakeUrls::new(Err(ApiError::Transport("down".into())));
    let (h, mut rx) = spawn(cfg(), urls.clone(), &tokio::runtime::Handle::current());
    let first = next_reason(&mut rx).await;
    assert!(first.starts_with("url fetch failed (transport)"), "{first}");
    assert!(first.contains("never connected"), "{first}");
    for _ in 0..11 {
        next_reason(&mut rx).await;
    }
    let t = urls.times.lock().unwrap().clone();
    let span = (t[t.len() - 1] - t[0]).as_secs_f64();
    // Eleven gaps of at least 1, 2, 4, 8, 16, 32 and five of 60 s (jitter only adds).
    assert!(span >= 362.9, "12 attempts in {span} s");
    h.shutdown();
}

/// The attempt after a resume whose URL fetch fails is a fresh one, not a resume.
#[tokio::test]
async fn a_failed_fetch_after_a_resume_is_logged_as_fresh() {
    let server = FakeServer::start(Vec::new()).await;
    let resume = format!(r#"{{"type":"reconnect_url","url":"{}"}}"#, server.url());
    server.push_script(Script {
        steps: vec![hello(), Step::Send(resume), Step::Drop],
        ..Script::default()
    });
    server.push_script(Script {
        steps: vec![hello(), Step::Drop],
        ..Script::default()
    });
    let urls = FakeUrls::new(Err(ApiError::Transport("down".into())));
    urls.queue.lock().unwrap().push_back(Ok(server.url()));
    let mut c = cfg();
    c.backoff_min = Duration::from_millis(10);
    let (h, mut rx) = spawn(c, urls.clone(), &tokio::runtime::Handle::current());
    assert!(next_reason(&mut rx).await.contains("fresh"));
    assert!(next_reason(&mut rx).await.contains("resume"));
    let third = next_reason(&mut rx).await;
    assert!(third.starts_with("url fetch failed"), "{third}");
    assert!(third.contains("fresh"), "{third}");
    h.shutdown();
}

/// A listener that accepts and never answers the handshake must not hold the
/// task: the attempt is dropped after `connect_timeout` and retried.
#[tokio::test(start_paused = true)]
async fn a_connect_that_never_answers_times_out() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((sock, _)) = listener.accept().await {
            held.push(sock);
        }
    });
    let urls = FakeUrls::new(Ok(Url::parse(&format!("ws://{addr}/")).unwrap()));
    let mut c = cfg();
    c.connect_timeout = Duration::from_secs(2);
    let (h, mut rx) = spawn(c, urls.clone(), &tokio::runtime::Handle::current());
    let first = next_reason(&mut rx).await;
    assert!(first.starts_with("connect timeout"), "{first}");
    assert!(first.contains("never connected"), "{first}");
    next_reason(&mut rx).await;
    assert!(urls.count() >= 2, "the attempt was retried");
    h.shutdown();
}

/// A send backlog queued while the socket is down does not hurry the reconnect:
/// attempts stay spaced by the growing backoff.
#[tokio::test]
async fn a_send_backlog_does_not_shorten_the_backoff() {
    let script = || Script {
        steps: vec![hello(), Step::Drop],
        ..Script::default()
    };
    let server = FakeServer::start((0..6).map(|_| script()).collect()).await;
    let urls = FakeUrls::new(Ok(server.url()));
    let mut c = cfg();
    c.backoff_min = Duration::from_millis(100);
    c.backoff_max = Duration::from_secs(5);
    let (h, mut rx) = spawn(c, urls.clone(), &tokio::runtime::Handle::current());
    for _ in 0..4 {
        next_reason(&mut rx).await;
        for i in 0..5 {
            h.send(Outgoing::Ping { id: 100 + i });
        }
    }
    let t = urls.times.lock().unwrap().clone();
    let third = (t[3] - t[2]).as_millis();
    assert!(third >= 380, "third gap {third} ms did not back off");
    h.shutdown();
}
