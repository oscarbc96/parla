//! `NativeCall`: one native huddle, from the join credentials to the leave.
//!
//! Provider-neutral. It takes a [`NativeCallJoin`] and two device factories
//! and runs two threads: the media thread of [`MediaPipeline`] and one engine
//! thread (a current-thread runtime) that owns the signalling websocket, the
//! sans-IO signalling machine, the `str0m` peer and the UDP socket.
//!
//! The engine never polls: it waits on the socket, the websocket, the frames
//! the pipeline produced and a wake-up from the handle, and sleeps until the
//! earliest of the ICE timer, the signalling machine's own deadline (ping and
//! handshake, absent while idle) and the connect or leave timeout. Nothing
//! queues without a bound: frames and events go through bounded channels and
//! are dropped (and counted by their owners) when a consumer is behind, and
//! mute and leave are two flags.

use super::pipeline::{
    EncodedFrame, EventOut, MediaPipeline, PacketOut, PendingPipeline, PipelineEvent, RtpPacket,
    SinkFactory, SourceFactory,
};
use chime_signal::{Config, Event as SignalEvent, Input, Machine, Output};
use domain::NativeCallJoin;
use futures::channel::mpsc as events_mpsc;
use futures::{SinkExt, StreamExt};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{
    Candidate, Event as RtcEvent, IceConnectionState, Input as RtcInput, Output as RtcOutput, Rtc,
};
use tokio::net::UdpSocket;
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message, client::IntoClientRequest, http::HeaderValue};
use zeroize::Zeroize;

/// Events queued for the consumer; beyond this new ones are dropped, except
/// the final one, which waits briefly.
const EVENT_QUEUE: usize = 64;
/// Encoded frames queued from the media thread to the engine.
const FRAME_QUEUE: usize = 32;
/// The signalling websocket must be up this soon.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Media must connect this long after the audio devices are up, or the join
/// failed.
const MEDIA_TIMEOUT: Duration = Duration::from_secs(15);
/// The audio devices must be open this long after the SDP answer (a stuck
/// audio stack, or a permission prompt nobody answers, fails the join).
#[cfg(not(test))]
const DEVICE_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const DEVICE_TIMEOUT: Duration = Duration::from_millis(500);
/// Resolving the audio host and finding the route to it.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
/// One signalling frame must be written this soon.
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
/// The best-effort LEAVE after a failure may take this long.
const FAREWELL_WAIT: Duration = Duration::from_millis(300);
/// How long a leave waits for the server's ack before closing anyway.
const LEAVE_WAIT: Duration = Duration::from_secs(1);
/// How long closing the websocket may take.
const CLOSE_WAIT: Duration = Duration::from_millis(300);
/// How long the handle waits for the engine thread after a leave request.
const JOIN_WITHIN: Duration = Duration::from_secs(2);
/// How long the final event may wait for room in a full queue.
const FINAL_EVENT_WAIT: Duration = Duration::from_millis(500);
/// A received sequence number this far from the last one is not the same
/// stream (a new SSRC starts anywhere); jitter and reordering stay well below.
const SEQ_JUMP: u16 = 3_000;
/// Reordering never exceeds this many packets; a larger sequence step must
/// also keep the RTP clock consistent to count as the same stream.
const REORDER: u16 = 100;
/// RTP clock deviation (one second at 48 kHz) from what the sequence step
/// predicts that marks a different stream.
const TS_JUMP: u32 = 48_000;
/// Samples per Opus frame, the RTP clock step of one packet.
const TS_STEP: i32 = 960;
/// Devices for the pipeline (opened on its media thread).
pub struct Devices {
    pub source: SourceFactory,
    pub sink: SinkFactory,
}

/// One attendee as the call reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Participant {
    pub attendee_id: String,
    pub external_user_id: String,
    pub muted: bool,
    pub dropped: bool,
    pub stream_id: u32,
}

/// What a running call tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallEvent {
    /// Signalling and media are being set up.
    Connecting,
    /// Media is up: ICE and DTLS completed.
    Live,
    /// The local mute state was applied.
    Muted(bool),
    /// The attendee list changed (people only: a content share attendee is
    /// reported as [`CallEvent::ScreenShare`]).
    Participants(Vec<Participant>),
    /// The attendees speaking now, by attendee id. Sent only when the set
    /// changes, never per audio frame.
    Speaking(Vec<String>),
    /// Someone started (`true`) or stopped sharing their screen. Sent only on
    /// a change.
    ScreenShare(bool),
    /// No microphone could be opened; the call is receive-only.
    CaptureUnavailable(String),
    /// The microphone stream failed mid-call; the owner may reopen it.
    CaptureError(String),
    /// The speaker stream failed mid-call; the owner may reopen it.
    PlaybackError(String),
    /// The call ended with an error. `before_media` is true when media never
    /// came up, so the caller may fall back to another way to join.
    Failed { before_media: bool, reason: String },
    /// The call ended after a leave (or the server closed it cleanly).
    Left,
}

/// Receives [`CallEvent`]s; the last one is `Failed` or `Left`, then the
/// stream ends.
pub type CallEvents = events_mpsc::Receiver<CallEvent>;

/// A device reopen the owner asked for: the new microphone, speaker or both.
type Reopen = (Option<SourceFactory>, Option<SinkFactory>);

#[derive(Default)]
struct Shared {
    muted: AtomicBool,
    leave: AtomicBool,
    /// The latest reopen request; a newer one replaces it per side.
    reopen: std::sync::Mutex<Option<Reopen>>,
    wake: Notify,
}

/// The attendee id suffix of a content (screen) share in the media service.
const CONTENT_SUFFIX: &str = "#content";

/// Splits the attendee list into people and whether a share is running.
fn split_people(list: Vec<chime_signal::Participant>) -> (Vec<Participant>, bool) {
    let mut sharing = false;
    let people = list
        .into_iter()
        .filter(|p| {
            let content = p.attendee_id.ends_with(CONTENT_SUFFIX);
            sharing |= content && !p.dropped;
            !content
        })
        .map(|p| Participant {
            attendee_id: p.attendee_id,
            external_user_id: p.external_user_id,
            muted: p.muted,
            dropped: p.dropped,
            stream_id: p.stream_id,
        })
        .collect();
    (people, sharing)
}

/// Who is in the call, who speaks and whether a share runs; turns signalling
/// events into [`CallEvent`]s, each sent only on a change.
#[derive(Default)]
struct Roster {
    people: Vec<Participant>,
    sharing: bool,
    /// Speaking streams, as last reported.
    streams: Vec<u32>,
    /// Speaking attendees, as last sent.
    names: Vec<String>,
}

impl Roster {
    fn on_people(&mut self, list: Vec<chime_signal::Participant>) -> Vec<CallEvent> {
        let (people, sharing) = split_people(list);
        self.people = people.clone();
        let mut out = vec![CallEvent::Participants(people)];
        if sharing != self.sharing {
            self.sharing = sharing;
            out.push(CallEvent::ScreenShare(sharing));
        }
        // The list may name a speaker it could not before, or drop one.
        out.extend(self.speakers());
        out
    }

    fn on_speaking(&mut self, streams: Vec<u32>) -> Vec<CallEvent> {
        self.streams = streams;
        self.speakers().into_iter().collect()
    }

    fn speakers(&mut self) -> Option<CallEvent> {
        let names = speaking_attendees(&self.streams, &self.people);
        (names != self.names).then(|| {
            self.names = names.clone();
            CallEvent::Speaking(names)
        })
    }
}

/// The attendee ids of the speaking `streams`, as the last list names them.
fn speaking_attendees(streams: &[u32], people: &[Participant]) -> Vec<String> {
    people
        .iter()
        .filter(|p| !p.dropped && streams.contains(&p.stream_id))
        .map(|p| p.attendee_id.clone())
        .collect()
}

/// Handle to a running call. Dropping it leaves the call.
pub struct NativeCall {
    shared: Arc<Shared>,
    done: std::sync::mpsc::Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl NativeCall {
    /// Starts joining. Returns at once; progress arrives as events.
    pub fn join(join: NativeCallJoin, devices: Devices) -> (Self, CallEvents) {
        let (mut events_tx, events_rx) = events_mpsc::channel(EVENT_QUEUE);
        let (done_tx, done) = std::sync::mpsc::sync_channel(1);
        let shared = Arc::new(Shared::default());
        let engine = Engine {
            join,
            devices: Some(devices),
            shared: shared.clone(),
            events: events_tx.clone(),
            live: false,
            last_error: None,
            roster: Roster::default(),
        };
        let spawned = std::thread::Builder::new()
            .name("parla-call".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt.block_on(engine.run()),
                    Err(e) => {
                        let _ = engine.events.clone().try_send(CallEvent::Failed {
                            before_media: true,
                            reason: format!("runtime: {e}"),
                        });
                    }
                }
                let _ = done_tx.send(());
            });
        let thread = match spawned {
            Ok(t) => Some(t),
            Err(e) => {
                let _ = events_tx.try_send(CallEvent::Failed {
                    before_media: true,
                    reason: format!("call thread: {e}"),
                });
                None
            }
        };
        (
            Self {
                shared,
                done,
                thread,
            },
            events_rx,
        )
    }

    /// Mutes or unmutes the microphone: the pipeline gate and the server's
    /// audio control. Coalesces: only the latest value matters.
    pub fn mute(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::Relaxed);
        self.shared.wake.notify_one();
    }

    /// Reopens the microphone (`source`), the speaker (`sink`) or both: a
    /// device change, a stream error or a new device choice. Coalesces: a
    /// later request replaces an earlier one that has not run yet, per side.
    pub fn replace_devices(&self, source: Option<SourceFactory>, sink: Option<SinkFactory>) {
        if let Ok(mut slot) = self.shared.reopen.lock() {
            let (old_source, old_sink) = slot.take().unwrap_or((None, None));
            *slot = Some((source.or(old_source), sink.or(old_sink)));
        }
        self.shared.wake.notify_one();
    }

    /// Leaves: sends the leave frame, waits briefly for the ack, closes the
    /// connection and stops the media thread. Returns once both threads are
    /// gone, at most about two seconds later.
    pub fn leave(mut self) {
        self.shutdown();
    }

    /// Leaves without waiting: asks the engine to leave and returns at once.
    /// The engine and media threads finish on their own (the owner learns of
    /// the end from [`CallEvent::Left`]). Use it where blocking, even for a
    /// moment, is not acceptable, such as a UI thread; [`NativeCall::leave`]
    /// and dropping the handle wait up to about two seconds for the threads.
    pub fn leave_detached(mut self) {
        self.shared.leave.store(true, Ordering::Relaxed);
        self.shared.wake.notify_one();
        // Detach: dropping a join handle leaves the thread running.
        drop(self.thread.take());
    }

    fn shutdown(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.shared.leave.store(true, Ordering::Relaxed);
        self.shared.wake.notify_one();
        // Disconnected means the thread already ended; only a timeout leaves
        // it running, and then it is detached rather than blocking the owner.
        if !matches!(
            self.done.recv_timeout(JOIN_WITHIN),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ) {
            let _ = thread.join();
        } else {
            tracing::warn!("call thread did not stop within {JOIN_WITHIN:?}");
        }
    }
}

impl Drop for NativeCall {
    fn drop(&mut self) {
        self.shutdown();
    }
}

enum End {
    Left,
    Failed(String),
}

struct Engine {
    join: NativeCallJoin,
    devices: Option<Devices>,
    shared: Arc<Shared>,
    events: events_mpsc::Sender<CallEvent>,
    /// Media came up (the failure is then not "before media").
    live: bool,
    /// Status of the last error frame the server sent, for the failure text.
    last_error: Option<u32>,
    roster: Roster,
}

impl Engine {
    fn emit(&mut self, event: CallEvent) {
        // A consumer that is behind loses state updates; the next one
        // carries the whole state anyway.
        let _ = self.events.try_send(event);
    }

    async fn run(mut self) {
        self.emit(CallEvent::Connecting);
        let end = self.session().await;
        let last = match end {
            End::Left => CallEvent::Left,
            // The owner asked to leave: whatever broke on the way out is not
            // a failure.
            End::Failed(_) if self.shared.leave.load(Ordering::Relaxed) => CallEvent::Left,
            End::Failed(reason) => CallEvent::Failed {
                before_media: !self.live,
                reason: match self.last_error {
                    Some(status) => format!("{reason} (last server error {status})"),
                    None => reason,
                },
            },
        };
        let _ = tokio::time::timeout(FINAL_EVENT_WAIT, self.events.send(last)).await;
    }

    /// Runs a pending device reopen: on the running pipeline, or on the
    /// devices not opened yet; while they are opening it waits for them.
    fn apply_reopen(&mut self, rtp: &Rtp) {
        let ready = rtp.pipeline.is_some() || self.devices.is_some();
        if !ready {
            return;
        }
        let Some((source, sink)) = self.shared.reopen.lock().ok().and_then(|mut s| s.take()) else {
            return;
        };
        if let Some(p) = &rtp.pipeline {
            if let Err((source, sink)) = p.replace_devices(source, sink) {
                // The media queue was full: put the request back for the next
                // wake-up (packets arrive every 20 ms). A newer request that
                // came in meanwhile wins, side by side.
                if let Ok(mut slot) = self.shared.reopen.lock() {
                    let (new_source, new_sink) = slot.take().unwrap_or((None, None));
                    *slot = Some((new_source.or(source), new_sink.or(sink)));
                }
            }
        } else if let Some(d) = self.devices.as_mut() {
            if let Some(s) = source {
                d.source = s;
            }
            if let Some(s) = sink {
                d.sink = s;
            }
        }
    }

    /// Resolves when the owner asks to leave.
    async fn leave_requested(shared: &Shared) {
        while !shared.leave.load(Ordering::Relaxed) {
            shared.wake.notified().await;
        }
    }

    async fn session(&mut self) -> End {
        let url = if self.join.signaling_url.contains('?') {
            format!(
                "{}&X-Chime-Control-Protocol-Version=3&X-Amzn-Chime-Send-Close-On-Error=1",
                self.join.signaling_url
            )
        } else {
            format!(
                "{}?X-Chime-Control-Protocol-Version=3&X-Amzn-Chime-Send-Close-On-Error=1",
                self.join.signaling_url
            )
        };
        let Ok(mut request) = url.into_client_request() else {
            return End::Failed("signalling address is not a websocket request".into());
        };
        let mut protocol = format!("_aws_wt_session, {}", self.join.join_token.expose());
        let header = HeaderValue::from_str(&protocol);
        protocol.zeroize();
        let Ok(mut header) = header else {
            return End::Failed("join token is not usable".into());
        };
        header.set_sensitive(true);
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", header);
        let shared = self.shared.clone();
        let connected = tokio::select! {
            r = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request)) => r,
            () = Self::leave_requested(&shared) => return End::Left,
        };
        let ws = match connected {
            Ok(Ok((ws, _response))) => ws,
            Ok(Err(e)) => return End::Failed(describe_ws_error(&e)),
            Err(_) => return End::Failed("signalling connection timed out".into()),
        };
        let (mut ws_tx, mut ws_rx) = ws.split();
        let end = self.drive(&mut ws_tx, &mut ws_rx).await;
        let _ = tokio::time::timeout(CLOSE_WAIT, ws_tx.close()).await;
        end
    }

    async fn drive<Tx, Rx>(&mut self, ws_tx: &mut Tx, ws_rx: &mut Rx) -> End
    where
        Tx: futures::Sink<Message> + Unpin,
        Rx: futures::Stream<Item = Result<Message, tungstenite::Error>> + Unpin,
    {
        let shared = self.shared.clone();
        // Media socket on the local address the OS routes to the audio host.
        let host = self.join.audio_host_url.to_string();
        let resolved = bounded(
            &shared,
            RESOLVE_TIMEOUT,
            "resolving the audio host",
            async {
                tokio::net::lookup_host(host)
                    .await
                    .ok()
                    .and_then(|mut a| a.find(SocketAddr::is_ipv4))
            },
        )
        .await;
        let audio_addr = match resolved {
            Ok(Some(a)) => a,
            Ok(None) => return End::Failed("audio host did not resolve".into()),
            Err(end) => return end,
        };
        let routed = bounded(
            &shared,
            RESOLVE_TIMEOUT,
            "finding the route to the audio host",
            local_ip_towards(audio_addr),
        )
        .await;
        let local_ip = match routed {
            Ok(Ok(ip)) => ip,
            Ok(Err(e)) => return End::Failed(format!("no route to the audio host: {e}")),
            Err(end) => return end,
        };
        let sock = match UdpSocket::bind((local_ip, 0)).await {
            Ok(s) => s,
            Err(e) => return End::Failed(format!("media socket: {e}")),
        };
        let local = match sock.local_addr() {
            Ok(a) => a,
            Err(e) => return End::Failed(format!("media socket: {e}")),
        };

        let mut rtp = match Rtp::new(local) {
            Ok(r) => r,
            Err(e) => return End::Failed(e),
        };
        let offer = rtp.offer.take().unwrap_or_default();

        let now = Instant::now();
        let mut machine = Machine::new(
            Config {
                app_name: "parla".into(),
                app_version: env!("CARGO_PKG_VERSION").into(),
                platform_name: "native".into(),
                os_name: std::env::consts::OS.into(),
                os_version: "0".into(),
                audio_host: self.join.audio_host_url.to_string(),
                audio_session_id: None,
                attendee_id: self.join.attendee_id.to_string(),
            },
            now,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        );
        let mut out = machine.handle(now, Input::Opened);
        out.extend(machine.handle(now, Input::LocalOffer { sdp: offer }));

        let end = self
            .run_loop(&mut machine, &mut rtp, &sock, local, ws_tx, ws_rx, out)
            .await;
        // Any failure with the socket still up tells the server we are gone,
        // so the seat does not linger until its own timeout.
        if matches!(end, End::Failed(_)) {
            farewell(&mut machine, ws_tx).await;
        }
        if let Some(p) = rtp.pipeline.take() {
            p.stop();
        }
        end
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_loop<Tx, Rx>(
        &mut self,
        machine: &mut Machine,
        rtp: &mut Rtp,
        sock: &UdpSocket,
        local: SocketAddr,
        ws_tx: &mut Tx,
        ws_rx: &mut Rx,
        mut out: Vec<Output>,
    ) -> End
    where
        Tx: futures::Sink<Message> + Unpin,
        Rx: futures::Stream<Item = Result<Message, tungstenite::Error>> + Unpin,
    {
        // Kept alive here so the receiver never reports "closed".
        let (frames_tx, mut frames_rx) = tokio::sync::mpsc::channel::<EncodedFrame>(FRAME_QUEUE);
        let mut muted = false;
        let mut leave_deadline: Option<Instant> = None;
        // The devices are opening (never awaited inline) until this resolves.
        let mut pending: Option<PendingPipeline> = None;
        let mut devices_deadline: Option<Instant> = None;
        // When the devices came up; the media timeout counts from here.
        let mut media_from: Option<Instant> = None;
        let mut buf = vec![0u8; 2_048];
        rtp.drain(sock).await;

        loop {
            let now = Instant::now();
            if self.shared.leave.load(Ordering::Relaxed) && leave_deadline.is_none() {
                leave_deadline = Some(now + LEAVE_WAIT);
                out.extend(machine.handle(now, Input::Leave));
            }
            let want = self.shared.muted.load(Ordering::Relaxed);
            if want != muted {
                muted = want;
                if let Some(p) = &rtp.pipeline {
                    p.set_muted(muted);
                }
                out.extend(machine.handle(now, Input::SetMuted(muted)));
                self.emit(CallEvent::Muted(muted));
            }
            self.apply_reopen(rtp);

            for o in std::mem::take(&mut out) {
                match o {
                    Output::Send(bytes) => {
                        if let Err(end) =
                            send_frame(&self.shared, ws_tx, bytes, leave_deadline.is_some()).await
                        {
                            return end;
                        }
                    }
                    Output::Event(SignalEvent::RemoteAnswer { sdp }) => {
                        if let Err(e) = rtp.accept_answer(&sdp) {
                            return End::Failed(e);
                        }
                        let packets = frames_tx.clone();
                        let packet_out: PacketOut = Box::new(move |f| {
                            let _ = packets.try_send(f);
                        });
                        let mut events = self.events.clone();
                        let event_out: EventOut = Box::new(move |e| {
                            let _ = events.try_send(match e {
                                PipelineEvent::CaptureUnavailable(m) => {
                                    CallEvent::CaptureUnavailable(m)
                                }
                                PipelineEvent::CaptureError(m) => CallEvent::CaptureError(m),
                                PipelineEvent::PlaybackError(m) => CallEvent::PlaybackError(m),
                            });
                        });
                        let Some(devices) = self.devices.take() else {
                            return End::Failed("call started twice".into());
                        };
                        match MediaPipeline::begin(
                            devices.source,
                            devices.sink,
                            packet_out,
                            event_out,
                        ) {
                            Ok(p) => {
                                pending = Some(p);
                                devices_deadline = Some(Instant::now() + DEVICE_TIMEOUT);
                            }
                            Err(e) => return End::Failed(format!("audio: {e}")),
                        }
                        rtp.drain(sock).await;
                    }
                    Output::Event(SignalEvent::Participants(list)) => {
                        for e in self.roster.on_people(list) {
                            self.emit(e);
                        }
                    }
                    Output::Event(SignalEvent::Speaking(streams)) => {
                        for e in self.roster.on_speaking(streams) {
                            self.emit(e);
                        }
                    }
                    Output::Event(SignalEvent::Error { status, .. }) => {
                        tracing::warn!("signalling error frame, status {status}");
                        self.last_error = Some(status);
                    }
                    Output::Event(SignalEvent::Left) => return End::Left,
                    Output::Event(SignalEvent::Failed(reason)) => return End::Failed(reason),
                    Output::Event(SignalEvent::TurnCredentials(_) | SignalEvent::Index { .. }) => {}
                }
            }

            if rtp.connected && rtp.pipeline.is_some() && !self.live {
                self.live = true;
                self.emit(CallEvent::Live);
            }
            if let Some(reason) = rtp.lost.take() {
                return End::Failed(reason);
            }
            let now = Instant::now();
            if let Some(d) = leave_deadline
                && now >= d
            {
                return End::Left;
            }
            if let Some(d) = devices_deadline
                && pending.is_some()
                && now >= d
            {
                return End::Failed("audio devices did not open in time".into());
            }
            if let Some(a) = media_from
                && !rtp.connected
                && now >= a + MEDIA_TIMEOUT
            {
                return End::Failed("media did not connect".into());
            }

            let mut wake = rtp.deadline;
            wake = wake.min(machine.next_deadline().unwrap_or(wake));
            if let Some(d) = leave_deadline {
                wake = wake.min(d);
            }
            if let (Some(a), false) = (media_from, rtp.connected) {
                wake = wake.min(a + MEDIA_TIMEOUT);
            }
            if let (Some(d), true) = (devices_deadline, pending.is_some()) {
                wake = wake.min(d);
            }
            tokio::select! {
                m = ws_rx.next() => {
                    let now = Instant::now();
                    match m {
                        Some(Ok(Message::Binary(b))) => {
                            out.extend(machine.handle(now, Input::Frame(b.to_vec())));
                        }
                        Some(Ok(Message::Close(frame))) => {
                            let normal = frame.is_some_and(|f| {
                                matches!(f.code, CloseCode::Normal | CloseCode::Away)
                            });
                            let input = if normal {
                                Input::ClosedNormally
                            } else {
                                Input::Closed
                            };
                            out.extend(machine.handle(now, input));
                        }
                        Some(Err(_)) | None => {
                            out.extend(machine.handle(now, Input::Closed));
                        }
                        Some(Ok(_)) => {}
                    }
                }
                r = sock.recv_from(&mut buf) => {
                    if let Ok((n, source)) = r {
                        rtp.receive(&buf[..n], source, local);
                        rtp.drain(sock).await;
                    }
                }
                ready = async {
                    match pending.as_mut() {
                        Some(p) => p.wait().await,
                        None => std::future::pending().await,
                    }
                }, if pending.is_some() => {
                    match ready {
                        Ok(()) => {
                            if let Some(p) = pending.take().and_then(PendingPipeline::into_pipeline) {
                                p.set_muted(muted);
                                rtp.pipeline = Some(p);
                                media_from = Some(Instant::now());
                                devices_deadline = None;
                                self.apply_reopen(rtp);
                            }
                        }
                        Err(e) => return End::Failed(format!("audio: {e}")),
                    }
                }
                frame = frames_rx.recv() => {
                    if let Some(frame) = frame {
                        rtp.write(frame);
                        rtp.drain(sock).await;
                    }
                }
                () = self.shared.wake.notified() => {}
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(wake)) => {
                    let now = Instant::now();
                    if machine.next_deadline().is_some_and(|d| now >= d) {
                        out.extend(machine.handle(now, Input::Tick));
                    }
                    let _ = rtp.rtc.handle_input(RtcInput::Timeout(now));
                    rtp.drain(sock).await;
                }
            }
        }
    }
}

/// Runs `fut` for at most `limit`, giving up early when the owner asks to
/// leave: a stuck step never keeps the engine from answering a leave.
async fn bounded<T>(
    shared: &Shared,
    limit: Duration,
    what: &str,
    fut: impl Future<Output = T>,
) -> Result<T, End> {
    tokio::select! {
        r = tokio::time::timeout(limit, fut) => {
            r.map_err(|_| End::Failed(format!("{what} timed out")))
        }
        () = Engine::leave_requested(shared) => Err(End::Left),
    }
}

/// Writes one signalling frame within a bound. Before a leave was requested
/// a request to leave also cuts a stuck write short; once leaving, only the
/// bound applies, so the LEAVE frame itself is not abandoned.
async fn send_frame<Tx>(
    shared: &Shared,
    ws_tx: &mut Tx,
    bytes: Vec<u8>,
    leaving: bool,
) -> Result<(), End>
where
    Tx: futures::Sink<Message> + Unpin,
{
    let message = Message::Binary(bytes.into());
    let lost = || End::Failed("signalling connection lost".into());
    if leaving {
        return match tokio::time::timeout(LEAVE_WAIT, ws_tx.send(message)).await {
            Ok(Ok(())) => Ok(()),
            _ => Err(lost()),
        };
    }
    match bounded(
        shared,
        SEND_TIMEOUT,
        "sending to the server",
        ws_tx.send(message),
    )
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(lost()),
        Err(end) => Err(end),
    }
}

/// Best-effort LEAVE after a failure, with the socket possibly broken: the
/// machine only sends it from a state that has something to leave.
async fn farewell<Tx>(machine: &mut Machine, ws_tx: &mut Tx)
where
    Tx: futures::Sink<Message> + Unpin,
{
    for o in machine.handle(Instant::now(), Input::Leave) {
        if let Output::Send(bytes) = o {
            let _ = tokio::time::timeout(FAREWELL_WAIT, ws_tx.send(Message::Binary(bytes.into())))
                .await;
        }
    }
}

/// The local address the OS would use to reach `peer` (no packet is sent).
async fn local_ip_towards(peer: SocketAddr) -> std::io::Result<std::net::IpAddr> {
    let probe = UdpSocket::bind("0.0.0.0:0").await?;
    probe.connect(peer).await?;
    Ok(probe.local_addr()?.ip())
}

/// A websocket error without anything the URL or headers carried.
fn describe_ws_error(e: &tungstenite::Error) -> String {
    match e {
        tungstenite::Error::Http(r) => {
            format!("signalling refused the connection (status {})", r.status())
        }
        tungstenite::Error::Io(_) => "signalling connection failed".into(),
        tungstenite::Error::Tls(_) => "signalling TLS failed".into(),
        _ => "signalling connection failed".into(),
    }
}

/// What to do with a received packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Part of the current stream.
    Push,
    /// Looks like the start of a different stream; held back (dropped)
    /// until a second packet confirms it.
    Hold,
    /// A new stream is confirmed: reset the jitter state, then push.
    Reset,
}

/// Detects a change of the remote stream (a new SSRC). `str0m` does not
/// expose the SSRC on its media events, so a change is inferred from the
/// sequence number and the RTP clock together, and only acted on once, after
/// two consecutive packets agree with each other but not with the old stream.
/// Interleaved streams never confirm, so they cannot make it thrash. A new
/// SSRC starts at a random sequence number and clock; the rare case where
/// both land within a hundred packets of the old stream goes unnoticed (the
/// jitter buffer then simply adopts the new numbers as ordinary loss).
#[derive(Default)]
struct StreamTracker {
    last: Option<(u16, u32)>,
    candidate: Option<(u16, u32)>,
}

/// True when a packet is consistent with the one before it.
fn continues(prev: (u16, u32), next: (u16, u32)) -> bool {
    let step = i32::from(next.0.wrapping_sub(prev.0) as i16);
    let distance = step.unsigned_abs();
    if distance > u32::from(SEQ_JUMP) {
        return false;
    }
    if distance <= u32::from(REORDER) {
        return true;
    }
    // A bigger step (a burst of loss) must keep the clock on track too.
    let clock = next.1.wrapping_sub(prev.1) as i32;
    clock
        .wrapping_sub(step.wrapping_mul(TS_STEP))
        .unsigned_abs()
        <= TS_JUMP
}

impl StreamTracker {
    fn observe(&mut self, seq: u16, ts: u32) -> Verdict {
        let now = (seq, ts);
        let Some(last) = self.last else {
            self.last = Some(now);
            return Verdict::Push;
        };
        if continues(last, now) {
            self.candidate = None;
            self.last = Some(now);
            return Verdict::Push;
        }
        match self.candidate {
            Some(c) if continues(c, now) => {
                self.candidate = None;
                self.last = Some(now);
                Verdict::Reset
            }
            _ => {
                self.candidate = Some(now);
                Verdict::Hold
            }
        }
    }
}

/// The `str0m` peer and what hangs off it.
struct Rtp {
    rtc: Rtc,
    mid: Mid,
    /// The offer, until the machine takes it.
    offer: Option<String>,
    pending: Option<SdpPendingOffer>,
    pipeline: Option<MediaPipeline>,
    connected: bool,
    /// Set when the media path died.
    lost: Option<String>,
    tracker: StreamTracker,
    deadline: Instant,
}

impl Rtp {
    fn new(local: SocketAddr) -> Result<Self, String> {
        let now = Instant::now();
        let mut rtc = Rtc::builder()
            .clear_codecs()
            .enable_opus(true, false)
            .build(now);
        let candidate = Candidate::host(local, "udp").map_err(|e| format!("candidate: {e}"))?;
        rtc.add_local_candidate(candidate);
        let mut api = rtc.sdp_api();
        let mid = api.add_media(MediaKind::Audio, Direction::SendRecv, None, None, None);
        let (offer, pending) = api.apply().ok_or("no offer to make")?;
        Ok(Self {
            rtc,
            mid,
            offer: Some(offer.to_sdp_string()),
            pending: Some(pending),
            pipeline: None,
            connected: false,
            lost: None,
            tracker: StreamTracker::default(),
            deadline: now,
        })
    }

    fn accept_answer(&mut self, sdp: &str) -> Result<(), String> {
        let answer = SdpAnswer::from_sdp_string(sdp).map_err(|_| "answer did not parse")?;
        let pending = self.pending.take().ok_or("answer without an offer")?;
        self.rtc
            .sdp_api()
            .accept_answer(pending, answer)
            .map_err(|e| format!("answer rejected: {e}"))
    }

    fn receive(&mut self, data: &[u8], source: SocketAddr, local: SocketAddr) {
        if let Ok(contents) = data.try_into() {
            let _ = self.rtc.handle_input(RtcInput::Receive(
                Instant::now(),
                Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: local,
                    contents,
                },
            ));
        }
    }

    /// Writes one encoded frame to the peer; dropped before media is up.
    fn write(&mut self, frame: EncodedFrame) {
        if !self.connected {
            return;
        }
        let Some(pt) = self
            .rtc
            .writer(self.mid)
            .and_then(|w| w.payload_params().next().map(|p| p.pt()))
        else {
            return;
        };
        if let Some(w) = self.rtc.writer(self.mid) {
            let time = MediaTime::new(u64::from(frame.timestamp), Frequency::FORTY_EIGHT_KHZ);
            let _ = w.write(pt, Instant::now(), time, frame.payload);
        }
    }

    /// Runs the peer to its next timeout (one mutation, then a full drain),
    /// sending what it transmits and handling what it reports.
    async fn drain(&mut self, sock: &UdpSocket) {
        loop {
            match self.rtc.poll_output() {
                Ok(RtcOutput::Timeout(t)) => {
                    self.deadline = t;
                    return;
                }
                Ok(RtcOutput::Transmit(t)) => {
                    if let Err(e) = sock.send_to(&t.contents, t.destination).await {
                        tracing::debug!("media send failed: {e}");
                    }
                }
                Ok(RtcOutput::Event(e)) => self.on_event(e),
                Err(e) => {
                    self.lost.get_or_insert(format!("media error: {e}"));
                    self.deadline = Instant::now() + Duration::from_secs(1);
                    return;
                }
            }
        }
    }

    fn on_event(&mut self, event: RtcEvent) {
        match event {
            RtcEvent::Connected => self.connected = true,
            RtcEvent::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                self.lost.get_or_insert("media connection lost".into());
            }
            RtcEvent::MediaData(d) => {
                let Some(p) = &self.pipeline else { return };
                let seq = **d.seq_range.end() as u16;
                match self.tracker.observe(seq, d.time.numer() as u32) {
                    Verdict::Hold => return,
                    Verdict::Reset => p.reset_stream(),
                    Verdict::Push => {}
                }
                p.push_packet(RtpPacket {
                    seq,
                    payload: d.data.to_vec(),
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    /// A packet `n` frames into a stream that starts at `seq0` and clock `ts0`.
    fn pkt(seq0: u16, ts0: u32, n: u32) -> (u16, u32) {
        (seq0.wrapping_add(n as u16), ts0.wrapping_add(n * 960))
    }

    fn feed(t: &mut StreamTracker, p: (u16, u32)) -> Verdict {
        t.observe(p.0, p.1)
    }

    #[test]
    fn small_steps_and_wraps_continue_the_stream() {
        assert!(continues((100, 0), (101, 960)));
        assert!(continues((100, 960), (90, 0)));
        assert!(continues((65_535, 0), (3, 4 * 960)));
        assert!(continues((3, 4 * 960), (65_530, 0)));
        // A silent gap (DTX) moves the clock a lot at a small step.
        assert!(continues((100, 0), (101, 480_000)));
    }

    #[test]
    fn a_loss_burst_on_the_same_stream_continues() {
        assert!(continues((100, 0), (600, 500 * 960)));
    }

    #[test]
    fn a_big_jump_or_a_clock_break_is_not_the_same_stream() {
        assert!(!continues((100, 0), (40_000, 40_000 * 960)));
        // A mid-size step (inside the old 3000 threshold) with a random clock.
        assert!(!continues((100, 0), (1_500, 3_000_000_000)));
    }

    #[test]
    fn a_new_stream_resets_once_after_two_agreeing_packets() {
        let mut t = StreamTracker::default();
        for n in 0..5 {
            assert_eq!(feed(&mut t, pkt(100, 5_000, n)), Verdict::Push);
        }
        let (s, c) = (30_000, 2_000_000_000);
        assert_eq!(feed(&mut t, pkt(s, c, 0)), Verdict::Hold);
        assert_eq!(feed(&mut t, pkt(s, c, 1)), Verdict::Reset);
        for n in 2..10 {
            assert_eq!(feed(&mut t, pkt(s, c, n)), Verdict::Push);
        }
    }

    #[test]
    fn a_mid_range_sequence_change_is_caught_by_the_clock() {
        let mut t = StreamTracker::default();
        assert_eq!(feed(&mut t, pkt(100, 5_000, 0)), Verdict::Push);
        // 1,500 packets away: the old seq-only rule (3,000) would miss it.
        let (s, c) = (1_600, 3_100_000_000);
        assert_eq!(feed(&mut t, pkt(s, c, 0)), Verdict::Hold);
        assert_eq!(feed(&mut t, pkt(s, c, 1)), Verdict::Reset);
    }

    #[test]
    fn interleaved_streams_never_reset() {
        let mut t = StreamTracker::default();
        let (a, b) = ((100u16, 5_000u32), (30_000u16, 2_000_000_000u32));
        assert_eq!(feed(&mut t, pkt(a.0, a.1, 0)), Verdict::Push);
        for n in 1..200 {
            let v = feed(&mut t, pkt(a.0, a.1, n));
            assert_eq!(v, Verdict::Push);
            let v = feed(&mut t, pkt(b.0, b.1, n));
            assert_eq!(v, Verdict::Hold);
        }
    }

    fn person(attendee: &str, stream: u32, dropped: bool) -> chime_signal::Participant {
        chime_signal::Participant {
            attendee_id: attendee.into(),
            external_user_id: format!("ext-{attendee}"),
            muted: false,
            dropped,
            stream_id: stream,
        }
    }

    #[test]
    fn a_content_share_attendee_is_a_share_not_a_person() {
        let (people, sharing) = split_people(vec![
            person("a1", 1, false),
            person("a2", 2, false),
            person("a2#content", 3, false),
        ]);
        assert!(sharing);
        assert_eq!(
            people
                .iter()
                .map(|p| p.attendee_id.as_str())
                .collect::<Vec<_>>(),
            ["a1", "a2"]
        );
        // A dropped share has ended.
        let (_, sharing) =
            split_people(vec![person("a1", 1, false), person("a1#content", 3, true)]);
        assert!(!sharing);
    }

    #[test]
    fn speaking_streams_map_to_attendees() {
        let (people, _) = split_people(vec![person("a1", 1, false), person("a2", 2, false)]);
        assert_eq!(speaking_attendees(&[2, 9], &people), vec!["a2".to_string()]);
        assert!(speaking_attendees(&[], &people).is_empty());
    }

    #[test]
    fn a_new_attendee_list_names_speakers_it_could_not_before() {
        let mut r = Roster::default();
        // Stream 2 speaks before the list says whose it is.
        assert!(r.on_speaking(vec![2]).is_empty());
        let out = r.on_people(vec![person("a1", 1, false), person("a2", 2, false)]);
        assert!(
            out.contains(&CallEvent::Speaking(vec!["a2".into()])),
            "{out:?}"
        );
        // The same list again changes nothing.
        let out = r.on_people(vec![person("a1", 1, false), person("a2", 2, false)]);
        assert!(
            !out.iter().any(|e| matches!(e, CallEvent::Speaking(_))),
            "{out:?}"
        );
        // The speaker drops out: nobody speaks.
        let out = r.on_people(vec![person("a1", 1, false), person("a2", 2, true)]);
        assert!(out.contains(&CallEvent::Speaking(vec![])), "{out:?}");
    }
}
