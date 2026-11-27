//! The session websocket connection: keepalive, jittered reconnect, auth-end
//! detection. Nothing here logs; URLs, tokens and cookies never reach an event
//! reason or a `Debug` impl.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use backon::{BackoffBuilder, ExponentialBuilder};
use futures::future::BoxFuture;
use futures::{FutureExt, SinkExt, StreamExt};
use slack_api::{ApiError, SessionCookie};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use url::Url;

use crate::event::{Outgoing, SlackEvent, parse_frame};

/// Where the connect URL comes from; asked again on every (re)connect.
pub trait UrlSource: Send + Sync {
    fn fetch(&self) -> BoxFuture<'_, Result<Url, ApiError>>;
}

pub struct ConnConfig {
    pub cookie: SessionCookie,
    /// Handshake `User-Agent`; `None` omits it (the probe has not confirmed it is needed).
    pub user_agent: Option<String>,
    /// Handshake `Origin`; `None` omits it.
    pub origin: Option<String>,
    /// Further handshake headers (name, value); values are treated as secret.
    pub extra_headers: Vec<(String, String)>,
    pub silence_before_ping: Duration,
    pub pong_timeout: Duration,
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    /// A connection must stay up this long before the next failure restarts the
    /// backoff at its minimum; a server that says hello then hangs up keeps backing off.
    pub healthy_uptime: Duration,
    /// Least gap between the starts of two connection attempts that `retry_now`
    /// can produce, so a caller that asks for a retry on every failure (queued
    /// sends, the "Retry now" button) cannot turn the backoff into a tight loop.
    pub retry_floor: Duration,
    /// Longest a connection attempt may take before it counts as lost.
    pub connect_timeout: Duration,
    /// Debug recorder (`PARLA_RECORD`); frames are redacted before they are written.
    pub recorder: Option<Arc<redact::record::Recorder>>,
}

impl ConnConfig {
    pub fn new(cookie: SessionCookie) -> ConnConfig {
        ConnConfig {
            cookie,
            user_agent: Some(slack_api::DEFAULT_USER_AGENT.to_string()),
            origin: Some("https://app.slack.com".to_string()),
            extra_headers: Vec::new(),
            silence_before_ping: Duration::from_secs(60),
            pong_timeout: Duration::from_secs(10),
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(60),
            healthy_uptime: Duration::from_secs(30),
            retry_floor: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(20),
            recorder: None,
        }
    }
}

impl fmt::Debug for ConnConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.extra_headers.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("ConnConfig")
            .field("cookie", &"REDACTED")
            .field("user_agent", &self.user_agent)
            .field("origin", &self.origin)
            .field("extra_headers", &names)
            .field("silence_before_ping", &self.silence_before_ping)
            .field("pong_timeout", &self.pong_timeout)
            .field("backoff_min", &self.backoff_min)
            .field("backoff_max", &self.backoff_max)
            .field("retry_floor", &self.retry_floor)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

#[derive(Debug)]
pub enum ConnEvent {
    /// The first inbound frame of a connection arrived (the spike showed `hello` is
    /// not guaranteed, so any frame counts).
    Connected {
        reconnect: bool,
    },
    Event(SlackEvent),
    Disconnected {
        reason: String,
    },
    /// No further attempts are made.
    AuthExpired,
}

pub struct RealtimeHandle {
    cmd: mpsc::Sender<Outgoing>,
    next_id: Arc<AtomicU64>,
    stop: oneshot::Sender<()>,
    retry: Arc<tokio::sync::Notify>,
}

impl fmt::Debug for RealtimeHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RealtimeHandle")
    }
}

impl RealtimeHandle {
    /// Queue a frame; dropped if the socket is not connected when it is picked up.
    pub fn send(&self, out: Outgoing) {
        // Bounded: frames queued while offline are stale anyway, so a full queue drops.
        let _ = self.cmd.try_send(out);
    }

    /// Next id of the counter shared by all client frames (pings included).
    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Cuts the current backoff wait short, for an explicit user "Retry now"
    /// only (never from an automatic path such as the outbox): the next attempt
    /// starts at once, but not less than `retry_floor` after the previous one
    /// started. A no-op while connected or attempting.
    pub fn retry_now(&self) {
        self.retry.notify_one();
    }

    /// Stop the task; dropping the handle does the same.
    pub fn shutdown(self) {
        let _ = self.stop.send(());
    }
}

/// Frames the command queue holds before `send` drops the newest.
const CMD_QUEUE: usize = 256;

pub fn spawn(
    cfg: ConnConfig,
    urls: Arc<dyn UrlSource>,
    rt: &tokio::runtime::Handle,
) -> (RealtimeHandle, mpsc::Receiver<ConnEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(CMD_QUEUE);
    let (ev_tx, ev_rx) = mpsc::channel(256);
    let (stop_tx, stop_rx) = oneshot::channel();
    let next_id = Arc::new(AtomicU64::new(1));
    let retry = Arc::new(tokio::sync::Notify::new());
    rt.spawn(run(
        cfg,
        urls,
        cmd_rx,
        ev_tx,
        stop_rx,
        next_id.clone(),
        retry.clone(),
    ));
    (
        RealtimeHandle {
            cmd: cmd_tx,
            next_id,
            stop: stop_tx,
            retry,
        },
        ev_rx,
    )
}

enum End {
    Stop,
    Auth,
    Lost(String),
}

struct Ctx {
    cfg: ConnConfig,
    cmd: mpsc::Receiver<Outgoing>,
    tx: mpsc::Sender<ConnEvent>,
    next_id: Arc<AtomicU64>,
    /// When the current connection delivered its first frame.
    connected_at: Option<Instant>,
    /// Whether the current attempt used a saved resume URL.
    resumed: bool,
    /// Whether the current connection delivered anything besides its first frame
    /// (`hello`) and `reconnect_url` housekeeping; a pong counts.
    delivered: bool,
}

impl Ctx {
    fn record(&self, dir: redact::record::Direction, frame: &str) {
        if let Some(r) = &self.cfg.recorder {
            r.record_ws(dir, frame);
        }
    }

    /// False when the receiver is gone.
    async fn emit(&self, ev: ConnEvent) -> bool {
        self.tx.send(ev).await.is_ok()
    }
}

fn backoff(cfg: &ConnConfig) -> backon::ExponentialBackoff {
    ExponentialBuilder::default()
        .with_min_delay(cfg.backoff_min)
        .with_max_delay(cfg.backoff_max)
        .with_factor(2.0)
        .with_jitter()
        .without_max_times()
        .build()
}

async fn run(
    cfg: ConnConfig,
    urls: Arc<dyn UrlSource>,
    cmd: mpsc::Receiver<Outgoing>,
    tx: mpsc::Sender<ConnEvent>,
    mut stop: oneshot::Receiver<()>,
    next_id: Arc<AtomicU64>,
    retry: Arc<tokio::sync::Notify>,
) {
    let mut cx = Ctx {
        cfg,
        cmd,
        tx,
        next_id,
        connected_at: None,
        resumed: false,
        delivered: false,
    };
    let mut delays = backoff(&cx.cfg);
    let mut reconnect = false;
    // A `reconnect_url` is single use; it is a credential and never logged.
    let mut resume: Option<Url> = None;
    loop {
        let attempt_started = Instant::now();
        // An attempt that fails before it picks a URL is a fresh one.
        cx.resumed = false;
        cx.delivered = false;
        let attempt = async {
            let (url, from_resume) = match resume.take() {
                Some(u) => (u, true),
                None => (
                    urls.fetch().await.map_err(|e| {
                        if e.is_auth() {
                            End::Auth
                        } else {
                            End::Lost(format!("url fetch failed ({})", api_kind(&e)))
                        }
                    })?,
                    false,
                ),
            };
            cx.resumed = from_resume;
            session(&mut cx, url, from_resume, &mut reconnect, &mut resume).await
        };
        let end = tokio::select! {
            _ = &mut stop => return,
            r = attempt => match r { Ok(e) | Err(e) => e },
        };
        // Only a connection that stayed up long enough restarts the backoff.
        let up = cx.connected_at.take().map(|t| t.elapsed());
        // A resumed connection that said hello and then delivered nothing is a dead
        // session: its `reconnect_url` leads to the same place, so the next attempt
        // is a fresh connect, and the silent stay does not restart the backoff.
        let dead_resume = cx.resumed && !cx.delivered;
        if dead_resume {
            resume = None;
        } else if up.is_some_and(|u| u >= cx.cfg.healthy_uptime) {
            delays = backoff(&cx.cfg);
        }
        match end {
            End::Stop => return,
            End::Auth => {
                let _ = cx.emit(ConnEvent::AuthExpired).await;
                return;
            }
            End::Lost(reason) => {
                // One short, secret-free line for the log: why, fresh or resumed
                // URL, and how long the socket delivered frames.
                let reason = format!(
                    "{reason} ({}, {})",
                    if cx.resumed { "resume" } else { "fresh" },
                    match up {
                        Some(u) => format!("up {}s", u.as_secs()),
                        None => "never connected".to_string(),
                    }
                );
                if !cx.emit(ConnEvent::Disconnected { reason }).await {
                    return;
                }
            }
        }
        let wait = delays
            .next()
            .unwrap_or(cx.cfg.backoff_max)
            .min(cx.cfg.backoff_max);
        // A permit stored while no one waited is stale; drop it first.
        retry.notified().now_or_never();
        tokio::select! {
            _ = &mut stop => return,
            _ = tokio::time::sleep(wait) => {}
            _ = retry.notified() => {
                // A requested retry skips the backoff but not the floor.
                tokio::select! {
                    _ = &mut stop => return,
                    _ = tokio::time::sleep_until(attempt_started + cx.cfg.retry_floor) => {}
                }
            }
        }
    }
}

/// A coarse, secret-free name for an API error.
fn api_kind(e: &ApiError) -> String {
    match e {
        ApiError::Http { status } => format!("http {status}"),
        ApiError::RateLimited { .. } => "rate limited".to_string(),
        ApiError::Transport(_) => "transport".to_string(),
        ApiError::Slack { error, .. } => {
            format!("slack {}", error.chars().take(40).collect::<String>())
        }
        _ => "other".to_string(),
    }
}

/// A coarse, secret-free name for a websocket error.
fn ws_kind(e: &WsError) -> String {
    match e {
        WsError::Io(io) => format!("io {:?}", io.kind()),
        WsError::Tls(_) => "tls".to_string(),
        WsError::Protocol(_) => "protocol".to_string(),
        WsError::Capacity(_) => "capacity".to_string(),
        WsError::Utf8(_) => "utf8".to_string(),
        WsError::ConnectionClosed | WsError::AlreadyClosed => "closed".to_string(),
        _ => "other".to_string(),
    }
}

fn request(
    cfg: &ConnConfig,
    url: &Url,
) -> Result<impl tokio_tungstenite::tungstenite::client::IntoClientRequest, String> {
    let mut req = url
        .as_str()
        .into_client_request()
        .map_err(|_| "bad websocket request".to_string())?;
    let h = req.headers_mut();
    let hv = |s: &str| HeaderValue::from_str(s).map_err(|_| "bad header value".to_string());
    h.insert("Cookie", hv(&cfg.cookie.header_value())?);
    if let Some(o) = &cfg.origin {
        h.insert("Origin", hv(o)?);
    }
    if let Some(ua) = &cfg.user_agent {
        h.insert("User-Agent", hv(ua)?);
    }
    for (k, v) in &cfg.extra_headers {
        let name = tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(k.as_bytes())
            .map_err(|_| "bad header name".to_string())?;
        h.insert(name, hv(v)?);
    }
    Ok(req)
}

fn is_auth_text(s: &str) -> bool {
    slack_api::AUTH_ERRORS.iter().any(|a| s.contains(a))
}

/// One connection attempt and, if it connects, its lifetime. `Ok` and `Err` both
/// carry the reason the attempt ended.
async fn session(
    cx: &mut Ctx,
    url: Url,
    from_resume: bool,
    reconnect: &mut bool,
    resume: &mut Option<Url>,
) -> Result<End, End> {
    let req = request(&cx.cfg, &url).map_err(End::Lost)?;
    // A handshake that hangs would otherwise hold the task (and every send) forever.
    let connected = tokio::time::timeout(
        cx.cfg.connect_timeout,
        tokio_tungstenite::connect_async(req),
    )
    .await;
    let Ok(connected) = connected else {
        return Err(End::Lost("connect timeout".to_string()));
    };
    let (mut ws, _resp) = match connected {
        Ok(ok) => ok,
        // A saved `reconnect_url` goes stale after a long sleep: that 401 says nothing
        // about the session, so fall back to a fresh URL instead of expiring it.
        Err(WsError::Http(resp)) if resp.status().as_u16() == 401 && from_resume => {
            return Err(End::Lost("reconnect url rejected".to_string()));
        }
        Err(WsError::Http(resp)) if resp.status().as_u16() == 401 => return Err(End::Auth),
        Err(WsError::Http(resp)) => {
            return Err(End::Lost(format!(
                "handshake rejected: http {}",
                resp.status().as_u16()
            )));
        }
        Err(e) => return Err(End::Lost(format!("connect failed ({})", ws_kind(&e)))),
    };
    // Frames queued while disconnected are stale (the domain re-subscribes on
    // `Connected { reconnect: true }`).
    while cx.cmd.try_recv().is_ok() {}

    let mut announced = false;
    let mut first_frame = false;
    let mut last_rx = Instant::now();
    let mut ping_sent: Option<Instant> = None;
    loop {
        let (deadline, awaiting) = match ping_sent {
            Some(t) => (t + cx.cfg.pong_timeout, true),
            None => (last_rx + cx.cfg.silence_before_ping, false),
        };
        tokio::select! {
            out = cx.cmd.recv() => {
                let Some(out) = out else { return Ok(End::Stop) };
                for frame in out.to_frames() {
                    cx.record(redact::record::Direction::Send, &frame);
                    if ws.send(Message::text(frame)).await.is_err() {
                        return Ok(End::Lost("write failed".to_string()));
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                if awaiting {
                    return Ok(End::Lost("pong timeout".to_string()));
                }
                let id = cx.next_id.fetch_add(1, Ordering::Relaxed);
                let ping = Outgoing::Ping { id }.to_frame();
                cx.record(redact::record::Direction::Send, &ping);
                if ws.send(Message::text(ping)).await.is_err() {
                    return Ok(End::Lost("write failed".to_string()));
                }
                ping_sent = Some(Instant::now());
            }
            msg = ws.next() => {
                let msg = match msg {
                    None => return Ok(End::Lost("connection closed".to_string())),
                    Some(Err(e)) => {
                        return Ok(End::Lost(format!("connection error ({})", ws_kind(&e))));
                    }
                    Some(Ok(m)) => m,
                };
                last_rx = Instant::now();
                ping_sent = None;
                match msg {
                    Message::Text(text) => {
                        cx.record(redact::record::Direction::Receive, &text);
                        if !announced {
                            announced = true;
                            first_frame = true;
                            cx.connected_at = Some(Instant::now());
                            if !cx.emit(ConnEvent::Connected { reconnect: *reconnect }).await {
                                return Ok(End::Stop);
                            }
                            *reconnect = true;
                        }
                        let ev = match parse_frame(&text) {
                            Ok(ev) => ev,
                            // Keep a well-formed frame we could not read visible as `Unknown`.
                            Err(_) => match unreadable(&text) {
                                Some(ev) => ev,
                                None => continue,
                            },
                        };
                        if first_frame {
                            first_frame = false;
                        } else if !matches!(ev, SlackEvent::ReconnectUrl { .. }) {
                            cx.delivered = true;
                        }
                        match &ev {
                            SlackEvent::ReconnectUrl { url } => {
                                *resume = Url::parse(url).ok();
                                continue;
                            }
                            SlackEvent::Goodbye => {
                                return Ok(End::Lost("server goodbye".to_string()));
                            }
                            SlackEvent::Error { msg, .. } if is_auth_text(msg) => {
                                return Ok(End::Auth);
                            }
                            _ => {}
                        }
                        if !cx.emit(ConnEvent::Event(ev)).await {
                            return Ok(End::Stop);
                        }
                    }
                    Message::Close(frame) => {
                        if let Some(f) = frame {
                            let code: u16 = f.code.into();
                            if is_auth_text(&f.reason) {
                                return Ok(End::Auth);
                            }
                            return Ok(End::Lost(format!("closed by server ({code})")));
                        }
                        return Ok(End::Lost("closed by server".to_string()));
                    }
                    _ => {}
                }
            }
        }
    }
}

/// A JSON frame `parse_frame` rejected, kept as an `Unknown` so it is not lost silently.
fn unreadable(text: &str) -> Option<SlackEvent> {
    let raw = serde_json::value::RawValue::from_string(text.to_string()).ok()?;
    let kind = serde_json::from_str::<serde_json::Value>(text)
        .ok()?
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unreadable")
        .to_string();
    Some(SlackEvent::Unknown { kind, raw })
}
