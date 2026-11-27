//! End-to-end tests of [`NativeCall`] without devices or the network: a fake
//! signalling server replays the real frame codec, and a second in-process
//! `str0m` peer plays the audio host (ICE-lite). Fake devices feed and
//! collect a 440 Hz tone.

use super::call::{CallEvent, CallEvents, Devices, NativeCall};
use super::pipeline::{
    AudioSink, AudioSource, CaptureTx, PipelineError, PlayoutTx, SinkFactory, SourceFactory,
};
use super::{FRAME_SAMPLES, FrameDecoder, FrameEncoder};
use chime_signal::proto::sdk_signal_frame::Type;
use chime_signal::proto::{
    SdkAudioStreamIdInfo, SdkAudioStreamIdInfoFrame, SdkIndexFrame, SdkJoinAckFrame,
    SdkSignalFrame, SdkSubscribeAckFrame,
};
use chime_signal::{decode, encode};
use domain::{NativeCallJoin, SecretText};
use futures::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use str0m::change::SdpOffer;
use str0m::media::{Frequency, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event as RtcEvent, Input as RtcInput, Output as RtcOutput, Rtc};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

/// Deadline for something that has to arrive over the fake network and the
/// media threads (10 s is plenty on an idle machine).
const WAIT: Duration = Duration::from_secs(10);

/// Deadline for a condition that needs real-time audio to flow (the fake
/// microphone and speaker each move a 20 ms frame every 20 ms). A shared CI
/// runner can be loaded for seconds at a time, so the frames arrive late; the
/// condition is still the condition, only the wait for it is longer.
const MEDIA_WAIT: Duration = Duration::from_secs(60);

fn tone(start: usize, n: usize) -> Vec<f32> {
    (start..start + n)
        .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin())
        .collect()
}

fn dominant_hz(pcm: &[f32]) -> f32 {
    let zc = pcm.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    zc as f32 / (pcm.len() as f32 / 48_000.0)
}

fn rms(pcm: &[f32]) -> f32 {
    (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len().max(1) as f32).sqrt()
}

// ---- fake devices ----

/// A microphone that delivers a 440 Hz tone in real time from its own thread.
struct ToneSource {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl AudioSource for ToneSource {
    fn start(&mut self, out: CaptureTx) -> Result<(), PipelineError> {
        let stop = self.stop.clone();
        self.thread = Some(std::thread::spawn(move || {
            let mut n = 0;
            while !stop.load(Ordering::Relaxed) {
                // A full ring drops the frame; the clock keeps going.
                let _ = out.send(&tone(n, FRAME_SAMPLES));
                n += FRAME_SAMPLES;
                std::thread::sleep(Duration::from_millis(20));
            }
        }));
        Ok(())
    }
}
impl Drop for ToneSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A speaker that consumes 20 ms every 20 ms and records what it was given.
struct ClockSink {
    played: Arc<Mutex<Vec<f32>>>,
    level: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl AudioSink for ClockSink {
    fn start(&mut self, demand: PlayoutTx) -> Result<(), PipelineError> {
        let (level, stop) = (self.level.clone(), self.stop.clone());
        self.thread = Some(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let l = level.load(Ordering::Relaxed);
                level.store(l.saturating_sub(FRAME_SAMPLES), Ordering::Relaxed);
                demand.request();
                std::thread::sleep(Duration::from_millis(20));
            }
        }));
        Ok(())
    }
    fn write(&mut self, pcm: &[f32]) {
        self.played.lock().unwrap().extend_from_slice(pcm);
        self.level.fetch_add(pcm.len(), Ordering::Relaxed);
    }
    fn queued_samples(&self) -> usize {
        self.level.load(Ordering::Relaxed)
    }
    fn delay_ms(&self) -> u32 {
        20
    }
}
impl Drop for ClockSink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn devices() -> (Devices, Arc<Mutex<Vec<f32>>>) {
    let played = Arc::new(Mutex::new(Vec::new()));
    let sink_played = played.clone();
    let source: SourceFactory = Box::new(|| {
        Ok(Box::new(ToneSource {
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }) as Box<dyn AudioSource>)
    });
    let sink: SinkFactory = Box::new(move || {
        Ok(Box::new(ClockSink {
            played: sink_played,
            level: Arc::new(AtomicUsize::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }) as Box<dyn AudioSink>)
    });
    (Devices { source, sink }, played)
}

// ---- fake audio host (ICE-lite str0m peer) ----

#[derive(Default)]
struct HostStats {
    received: AtomicU64,
    payloads: Mutex<Vec<Vec<u8>>>,
    sending: AtomicBool,
}

struct AudioHost {
    addr: SocketAddr,
    offers: mpsc::Sender<(String, oneshot::Sender<String>)>,
    stats: Arc<HostStats>,
}

/// Starts the host: it answers the offer it is given, then sends a 440 Hz
/// tone and records what it receives.
async fn audio_host() -> AudioHost {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sock.local_addr().unwrap();
    let (offers, mut offers_rx) = mpsc::channel::<(String, oneshot::Sender<String>)>(1);
    let stats = Arc::new(HostStats::default());
    stats.sending.store(true, Ordering::Relaxed);
    let task_stats = stats.clone();
    tokio::spawn(async move {
        let mut rtc = Rtc::builder()
            .clear_codecs()
            .enable_opus(true, false)
            .set_ice_lite(true)
            .build(Instant::now());
        rtc.add_local_candidate(Candidate::host(addr, "udp").unwrap());
        let Some((offer, reply)) = offers_rx.recv().await else {
            return;
        };
        let answer = rtc
            .sdp_api()
            .accept_offer(SdpOffer::from_sdp_string(&offer).unwrap())
            .unwrap();
        let _ = reply.send(answer.to_sdp_string());

        let mut encoder = FrameEncoder::new().unwrap();
        let mut mid: Option<Mid> = None;
        let mut connected = false;
        let (mut n, mut time) = (0usize, 0u64);
        let mut next_frame = tokio::time::Instant::now();
        let mut buf = vec![0u8; 2_048];
        loop {
            let deadline = loop {
                match rtc.poll_output() {
                    Ok(RtcOutput::Timeout(t)) => break t,
                    Ok(RtcOutput::Transmit(t)) => {
                        let _ = sock.send_to(&t.contents, t.destination).await;
                    }
                    Ok(RtcOutput::Event(RtcEvent::Connected)) => connected = true,
                    Ok(RtcOutput::Event(RtcEvent::MediaAdded(m))) => mid = Some(m.mid),
                    Ok(RtcOutput::Event(RtcEvent::MediaData(d))) => {
                        task_stats.received.fetch_add(1, Ordering::Relaxed);
                        task_stats.payloads.lock().unwrap().push(d.data.to_vec());
                    }
                    Ok(_) => {}
                    Err(_) => return,
                }
            };
            let sending = connected && task_stats.sending.load(Ordering::Relaxed);
            let wake = if sending {
                next_frame.min(tokio::time::Instant::from_std(deadline))
            } else {
                tokio::time::Instant::from_std(deadline)
            };
            tokio::select! {
                r = sock.recv_from(&mut buf) => {
                    if let Ok((len, source)) = r
                        && let Ok(contents) = buf[..len].try_into()
                    {
                        let _ = rtc.handle_input(RtcInput::Receive(Instant::now(), Receive {
                            proto: Protocol::Udp,
                            source,
                            destination: addr,
                            contents,
                        }));
                    }
                }
                () = tokio::time::sleep_until(wake) => {
                    let now = Instant::now();
                    if sending && tokio::time::Instant::now() >= next_frame {
                        if let Some(mid) = mid
                            && let Some(pt) = rtc.writer(mid).and_then(|w| w.payload_params().next().map(|p| p.pt()))
                            && let Some(w) = rtc.writer(mid)
                        {
                            let payload = encoder.encode(&tone(n, FRAME_SAMPLES)).unwrap();
                            let _ = w.write(pt, now, MediaTime::new(time, Frequency::FORTY_EIGHT_KHZ), payload);
                        }
                        n += FRAME_SAMPLES;
                        time += FRAME_SAMPLES as u64;
                        next_frame += Duration::from_millis(20);
                    }
                    let _ = rtc.handle_input(RtcInput::Timeout(now));
                }
            }
        }
    });
    AudioHost {
        addr,
        offers,
        stats,
    }
}

// ---- fake signalling server ----

#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    Normal,
    /// Closes the socket as soon as the join frame arrives.
    CloseOnJoin,
}

#[derive(Default)]
struct Seen {
    types: Mutex<Vec<Type>>,
    mutes: Mutex<Vec<bool>>,
    /// The mute value the subscribe frame carried.
    subscribe_muted: Mutex<Option<bool>>,
    /// Makes the server close the socket with a normal close code.
    close_normal: tokio::sync::Notify,
    /// Makes the server drop the connection without a close frame.
    drop_now: tokio::sync::Notify,
}

fn frame(ty: Type) -> SdkSignalFrame {
    SdkSignalFrame {
        timestamp_ms: 1,
        r#type: ty as i32,
        ..Default::default()
    }
}

fn bin(f: &SdkSignalFrame) -> Message {
    Message::Binary(encode(f).into())
}

/// Serves one connection; returns the signalling address.
async fn signalling(behaviour: Behaviour, host: Option<AudioHost>) -> (SocketAddr, Arc<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Seen::default());
    let task_seen = seen.clone();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        #[allow(clippy::result_large_err)] // the callback type is fixed by tungstenite
        let ws = tokio_tungstenite::accept_hdr_async(stream, |_: &Request, mut r: Response| {
            r.headers_mut()
                .insert("Sec-WebSocket-Protocol", "_aws_wt_session".parse().unwrap());
            Ok(r)
        })
        .await
        .unwrap();
        let (mut tx, mut rx) = ws.split();
        loop {
            let msg = tokio::select! {
                m = rx.next() => match m {
                    Some(Ok(m)) => m,
                    _ => return,
                },
                () = task_seen.close_normal.notified() => {
                    let _ = tx.send(Message::Close(Some(CloseFrame {
                        code: CloseCode::Normal,
                        reason: "".into(),
                    }))).await;
                    return;
                }
                () = task_seen.drop_now.notified() => return,
            };
            let Message::Binary(bytes) = msg else {
                continue;
            };
            let Ok(f) = decode(&bytes) else { continue };
            let Ok(ty) = Type::try_from(f.r#type) else {
                continue;
            };
            task_seen.types.lock().unwrap().push(ty);
            match ty {
                Type::Join => {
                    if behaviour == Behaviour::CloseOnJoin {
                        let _ = tx.close().await;
                        return;
                    }
                    let mut ack = frame(Type::JoinAck);
                    ack.joinack = Some(SdkJoinAckFrame::default());
                    let _ = tx.send(bin(&ack)).await;
                    let mut index = frame(Type::Index);
                    index.index = Some(SdkIndexFrame {
                        num_participants: Some(1),
                        ..Default::default()
                    });
                    let _ = tx.send(bin(&index)).await;
                }
                Type::Subscribe => {
                    *task_seen.subscribe_muted.lock().unwrap() =
                        f.sub.as_ref().and_then(|s| s.audio_muted);
                    let offer = f.sub.and_then(|s| s.sdp_offer).unwrap();
                    let host = host.as_ref().expect("a host to answer");
                    let (reply, answer) = oneshot::channel();
                    host.offers.send((offer, reply)).await.unwrap();
                    let mut ack = frame(Type::SubscribeAck);
                    ack.suback = Some(SdkSubscribeAckFrame {
                        sdp_answer: Some(answer.await.unwrap()),
                        ..Default::default()
                    });
                    let _ = tx.send(bin(&ack)).await;
                    let mut info = frame(Type::AudioStreamIdInfo);
                    info.audio_stream_id_info = Some(SdkAudioStreamIdInfoFrame {
                        streams: vec![SdkAudioStreamIdInfo {
                            audio_stream_id: Some(7),
                            attendee_id: Some("att-host".into()),
                            muted: Some(false),
                            external_user_id: Some("ext-host".into()),
                            dropped: Some(false),
                        }],
                    });
                    let _ = tx.send(bin(&info)).await;
                }
                Type::AudioControl => {
                    if let Some(c) = f.audio_control {
                        task_seen
                            .mutes
                            .lock()
                            .unwrap()
                            .push(c.muted.unwrap_or(false));
                    }
                }
                Type::Leave => {
                    let _ = tx.send(bin(&frame(Type::LeaveAck))).await;
                    let _ = tx.close().await;
                    return;
                }
                _ => {}
            }
        }
    });
    (addr, seen)
}

fn join_for(signal: SocketAddr, audio: SocketAddr) -> NativeCallJoin {
    NativeCallJoin {
        meeting_id: "meeting".into(),
        attendee_id: "att-self".into(),
        external_user_id: "ext-self".into(),
        join_token: SecretText::new("test-token"),
        signaling_url: format!("ws://{signal}/control").into(),
        audio_host_url: audio.to_string().into(),
        turn_control_url: None,
    }
}

async fn next(events: &mut CallEvents) -> CallEvent {
    tokio::time::timeout(WAIT, events.next())
        .await
        .expect("an event in time")
        .expect("the stream is open")
}

/// Reads events until one matches.
async fn until(events: &mut CallEvents, mut pick: impl FnMut(&CallEvent) -> bool) -> CallEvent {
    loop {
        let e = next(events).await;
        if pick(&e) {
            return e;
        }
    }
}

async fn wait_for(mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + MEDIA_WAIT;
    while !cond() {
        assert!(Instant::now() < end, "condition not met in time");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

struct Live {
    call: NativeCall,
    events: CallEvents,
    stats: Arc<HostStats>,
    seen: Arc<Seen>,
    played: Arc<Mutex<Vec<f32>>>,
    /// Events that arrived before `Live`.
    early: Vec<CallEvent>,
}

async fn live_call() -> Live {
    let host = audio_host().await;
    let (audio, stats) = (host.addr, host.stats.clone());
    let (signal, seen) = signalling(Behaviour::Normal, Some(host)).await;
    let (devices, played) = devices();
    let (call, mut events) = NativeCall::join(join_for(signal, audio), devices);
    assert_eq!(next(&mut events).await, CallEvent::Connecting);
    let mut early = Vec::new();
    until(&mut events, |e| {
        early.push(e.clone());
        *e == CallEvent::Live
    })
    .await;
    Live {
        call,
        events,
        stats,
        seen,
        played,
        early,
    }
}

#[tokio::test]
async fn a_tone_travels_both_ways_and_participants_arrive() {
    let Live {
        call,
        mut events,
        stats,
        played,
        early,
        ..
    } = live_call().await;

    // Participants arrive with the subscribe ack, before or after media.
    let people = match early
        .iter()
        .find(|e| matches!(e, CallEvent::Participants(_)))
    {
        Some(e) => e.clone(),
        None => until(&mut events, |e| matches!(e, CallEvent::Participants(_))).await,
    };
    let CallEvent::Participants(people) = people else {
        unreachable!()
    };
    assert_eq!(people.len(), 1);
    assert_eq!(people[0].attendee_id, "att-host");
    assert_eq!(people[0].stream_id, 7);

    // Host to client: the sink plays 440 Hz.
    wait_for(|| {
        let p = played.lock().unwrap();
        p.len() > 12 * FRAME_SAMPLES && {
            let tail = &p[p.len() - 8 * FRAME_SAMPLES..];
            rms(tail) > 0.1 && (dominant_hz(tail) - 440.0).abs() < 20.0
        }
    })
    .await;

    // Client to host. The host goes quiet first: with its tone playing out of
    // the speaker, the echo canceller rightly removes the same tone from the
    // microphone.
    stats.sending.store(false, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let from = stats.payloads.lock().unwrap().len();
    wait_for(|| stats.payloads.lock().unwrap().len() > from + 50).await;
    let payloads = stats.payloads.lock().unwrap()[from..].to_vec();
    let mut decoder = FrameDecoder::new().unwrap();
    let mut pcm = Vec::new();
    let mut out = vec![0.0; super::codec::MAX_DECODE_SAMPLES];
    for p in &payloads {
        let n = decoder.decode(p, &mut out).unwrap();
        pcm.extend_from_slice(&out[..n]);
    }
    let window = 8 * FRAME_SAMPLES;
    let loudest = pcm
        .chunks_exact(window)
        .max_by(|a, b| rms(a).total_cmp(&rms(b)))
        .unwrap();
    assert!(rms(loudest) > 0.05, "rms {}", rms(loudest));
    assert!((dominant_hz(loudest) - 440.0).abs() < 20.0);
    call.leave();
}

#[tokio::test]
async fn mute_stops_outbound_rtp_and_tells_the_server() {
    let Live {
        call,
        mut events,
        stats,
        seen,
        ..
    } = live_call().await;
    wait_for(|| stats.received.load(Ordering::Relaxed) > 10).await;

    call.mute(true);
    until(&mut events, |e| *e == CallEvent::Muted(true)).await;
    wait_for(|| seen.mutes.lock().unwrap().as_slice() == [true]).await;
    // Let frames already in flight arrive, then nothing more may.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let before = stats.received.load(Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(stats.received.load(Ordering::Relaxed), before);

    call.mute(false);
    until(&mut events, |e| *e == CallEvent::Muted(false)).await;
    wait_for(|| stats.received.load(Ordering::Relaxed) > before + 10).await;
    assert_eq!(seen.mutes.lock().unwrap().as_slice(), [true, false]);
    call.leave();
}

#[tokio::test]
async fn leave_sends_leave_and_joins_the_threads_within_two_seconds() {
    let Live {
        call,
        mut events,
        seen,
        ..
    } = live_call().await;
    let t = Instant::now();
    // `leave` blocks until the engine and media threads are joined.
    let leaver = tokio::task::spawn_blocking(move || call.leave());
    leaver.await.unwrap();
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert!(seen.types.lock().unwrap().contains(&Type::Leave));
    until(&mut events, |e| *e == CallEvent::Left).await;
    // The stream ends after the final event.
    assert!(events.next().await.is_none());
}

#[tokio::test]
async fn dropping_the_handle_leaves() {
    let Live { call, seen, .. } = live_call().await;
    let t = Instant::now();
    tokio::task::spawn_blocking(move || drop(call))
        .await
        .unwrap();
    assert!(t.elapsed() < Duration::from_secs(2));
    assert!(seen.types.lock().unwrap().contains(&Type::Leave));
}

#[tokio::test]
async fn a_failure_before_media_says_so() {
    let (signal, _seen) = signalling(Behaviour::CloseOnJoin, None).await;
    let audio: SocketAddr = "127.0.0.1:9".parse().unwrap();
    let (devices, _played) = devices();
    let (call, mut events) = NativeCall::join(join_for(signal, audio), devices);
    assert_eq!(next(&mut events).await, CallEvent::Connecting);
    let failed = until(&mut events, |e| matches!(e, CallEvent::Failed { .. })).await;
    assert!(matches!(
        failed,
        CallEvent::Failed {
            before_media: true,
            ..
        }
    ));
    call.leave();
}

#[tokio::test]
async fn an_unreachable_server_fails_before_media() {
    let free = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let signal = free.local_addr().unwrap();
    drop(free);
    let (devices, _played) = devices();
    let (call, mut events) =
        NativeCall::join(join_for(signal, "127.0.0.1:9".parse().unwrap()), devices);
    let failed = until(&mut events, |e| matches!(e, CallEvent::Failed { .. })).await;
    assert!(matches!(
        failed,
        CallEvent::Failed {
            before_media: true,
            ..
        }
    ));
    drop(call);
}

#[tokio::test]
async fn leaving_while_connecting_ends_with_left() {
    // A server that accepts the TCP connection and never answers.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let signal = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _held = listener.accept().await;
        tokio::time::sleep(Duration::from_secs(30)).await;
    });
    let (devices, _played) = devices();
    let (call, mut events) =
        NativeCall::join(join_for(signal, "127.0.0.1:9".parse().unwrap()), devices);
    assert_eq!(next(&mut events).await, CallEvent::Connecting);
    let t = Instant::now();
    tokio::task::spawn_blocking(move || call.leave())
        .await
        .unwrap();
    assert!(t.elapsed() < Duration::from_secs(2));
    until(&mut events, |e| *e == CallEvent::Left).await;
}

/// Devices whose speaker never opens (the factory blocks), as a hung audio
/// stack would.
fn hung_devices() -> (Devices, std::sync::mpsc::Sender<()>) {
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let source: SourceFactory = Box::new(|| Err(PipelineError::Device("no mic".into())));
    let sink: SinkFactory = Box::new(move || {
        let _ = hold.recv_timeout(Duration::from_secs(30));
        Err(PipelineError::Device("late".into()))
    });
    (Devices { source, sink }, release)
}

#[tokio::test]
async fn a_clean_server_close_while_joined_ends_with_left() {
    let Live {
        call,
        mut events,
        seen,
        ..
    } = live_call().await;
    seen.close_normal.notify_one();
    let last = until(&mut events, |e| {
        matches!(e, CallEvent::Left | CallEvent::Failed { .. })
    })
    .await;
    assert_eq!(last, CallEvent::Left);
    drop(call);
}

#[tokio::test]
async fn a_failure_after_live_is_not_before_media() {
    let Live {
        call,
        mut events,
        seen,
        ..
    } = live_call().await;
    seen.drop_now.notify_one();
    let failed = until(&mut events, |e| matches!(e, CallEvent::Failed { .. })).await;
    assert!(matches!(
        failed,
        CallEvent::Failed {
            before_media: false,
            ..
        }
    ));
    drop(call);
}

#[tokio::test]
async fn an_audio_start_failure_sends_leave_and_fails_before_media() {
    let host = audio_host().await;
    let audio = host.addr;
    let (signal, seen) = signalling(Behaviour::Normal, Some(host)).await;
    let source: SourceFactory = Box::new(|| Err(PipelineError::Device("no mic".into())));
    let sink: SinkFactory = Box::new(|| Err(PipelineError::Device("no speaker".into())));
    let (call, mut events) = NativeCall::join(join_for(signal, audio), Devices { source, sink });
    let failed = until(&mut events, |e| matches!(e, CallEvent::Failed { .. })).await;
    assert!(matches!(
        failed,
        CallEvent::Failed {
            before_media: true,
            ..
        }
    ));
    wait_for(|| seen.types.lock().unwrap().contains(&Type::Leave)).await;
    drop(call);
}

#[tokio::test]
async fn a_hung_device_open_fails_before_media_and_leaves() {
    let host = audio_host().await;
    let audio = host.addr;
    let (signal, seen) = signalling(Behaviour::Normal, Some(host)).await;
    let (devices, release) = hung_devices();
    let (call, mut events) = NativeCall::join(join_for(signal, audio), devices);
    let failed = until(&mut events, |e| matches!(e, CallEvent::Failed { .. })).await;
    assert!(matches!(
        failed,
        CallEvent::Failed {
            before_media: true,
            ..
        }
    ));
    wait_for(|| seen.types.lock().unwrap().contains(&Type::Leave)).await;
    tokio::task::spawn_blocking(move || drop(call))
        .await
        .unwrap();
    drop(release);
}

#[tokio::test]
async fn the_engine_answers_leave_while_a_device_hangs() {
    let host = audio_host().await;
    let audio = host.addr;
    let (signal, seen) = signalling(Behaviour::Normal, Some(host)).await;
    let (devices, release) = hung_devices();
    let (call, mut events) = NativeCall::join(join_for(signal, audio), devices);
    // The subscribe went out, so the devices are opening.
    wait_for(|| seen.types.lock().unwrap().contains(&Type::Subscribe)).await;
    let t = Instant::now();
    tokio::task::spawn_blocking(move || call.leave())
        .await
        .unwrap();
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    until(&mut events, |e| *e == CallEvent::Left).await;
    drop(release);
}

#[tokio::test]
async fn mute_before_the_join_rides_the_subscribe_frame() {
    let host = audio_host().await;
    let audio = host.addr;
    let (signal, seen) = signalling(Behaviour::Normal, Some(host)).await;
    let (devices, _played) = devices();
    let (call, mut events) = NativeCall::join(join_for(signal, audio), devices);
    call.mute(true);
    until(&mut events, |e| *e == CallEvent::Live).await;
    assert_eq!(*seen.subscribe_muted.lock().unwrap(), Some(true));
    call.leave();
}

#[tokio::test]
async fn leave_detached_returns_at_once_and_the_call_still_leaves() {
    let Live {
        call,
        mut events,
        seen,
        ..
    } = live_call().await;
    let t = Instant::now();
    call.leave_detached();
    assert!(t.elapsed() < Duration::from_millis(100));
    until(&mut events, |e| *e == CallEvent::Left).await;
    assert!(seen.types.lock().unwrap().contains(&Type::Leave));
}
