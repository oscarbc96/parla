//! The media thread and its handle.
//!
//! One thread exists from [`MediaPipeline::start`] to [`MediaPipeline::stop`].
//! It blocks on a single bounded channel that carries wake-ups from the
//! device callbacks, received packets from the network and the stop signal,
//! so it never polls and never wakes while no audio flows.
//!
//! Two independent clocks drive it, both owned by the devices:
//!
//! - the microphone callback fills a capture ring and wakes the thread, which
//!   cleans and encodes each full 20 ms frame;
//! - the speaker callback asks for more audio when its queue runs low, and the
//!   thread then decodes frames from the jitter buffer into the sink.
//!
//! Neither depends on the other: a missing, stalled or failed microphone never
//! stops remote audio.

use super::apm::EchoProcessor;
use super::codec::{FrameDecoder, FrameEncoder, MAX_DECODE_SAMPLES};
use super::jitter::{JitterBuffer, Playout};
use super::ring::Ring;
use super::{APM_SAMPLES, FRAME_SAMPLES};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::Duration;

/// Events queued to the media thread; beyond this, new packets are dropped
/// (and counted).
const QUEUE: usize = 128;
/// Capture ring size in samples (about 340 ms at 48 kHz); beyond it capture
/// is dropped (and counted).
const CAPTURE_RING: usize = 16_384;
/// The speaker queue level the media thread keeps: 40 ms at 48 kHz.
pub const SINK_TARGET_SAMPLES: usize = 1_920;
/// How long stopping the media thread may take before it is detached.
pub const STOP_WITHIN: Duration = Duration::from_secs(1);
/// Frames decoded per speaker request at most, so a stuck sink cannot spin.
const MAX_FRAMES_PER_WAKE: usize = 4;

/// Pipeline failure.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("audio device: {0}")]
    Device(String),
    #[error("codec: {0}")]
    Codec(String),
    #[error("media thread: {0}")]
    Thread(String),
}

impl From<opus::Error> for PipelineError {
    fn from(e: opus::Error) -> Self {
        Self::Codec(e.to_string())
    }
}

/// A received media packet, as the RTP layer hands it over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpPacket {
    pub seq: u16,
    pub payload: Vec<u8>,
}

/// An encoded 20 ms frame leaving the pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedFrame {
    /// 48 kHz sample clock of the frame start; advances 960 per frame, muted
    /// or not.
    pub timestamp: u32,
    pub payload: Vec<u8>,
}

/// Receives encoded frames; runs on the media thread, so it must not block:
/// hand the frame to a bounded channel with `try_send` and return.
pub type PacketOut = Box<dyn FnMut(EncodedFrame) + Send>;

/// Something the worker should know about the audio devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineEvent {
    /// No microphone could be opened; the call runs receive-only.
    CaptureUnavailable(String),
    /// The microphone stream failed while running (unplugged, device
    /// changed); the worker may reopen it.
    CaptureError(String),
    /// The speaker stream failed while running, or a reopened speaker did
    /// not start; the worker may reopen it.
    PlaybackError(String),
}

/// Receives [`PipelineEvent`]s; runs on the media thread, so it must not
/// block.
pub type EventOut = Box<dyn FnMut(PipelineEvent) + Send>;

/// Counters for logs and tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PipelineStats {
    /// Captured samples dropped because the capture ring was full.
    pub capture_dropped: u64,
    /// Received packets dropped because the queue was full.
    pub packets_dropped: u64,
    /// The microphone never opened.
    pub capture_unavailable: bool,
}

enum Event {
    /// Captured audio is waiting in the ring.
    Capture,
    /// The speaker wants more audio.
    Playout,
    /// A received packet and the stream generation it belongs to; a new
    /// generation makes the worker forget the jitter state first.
    Packet(RtpPacket, u32),
    /// A stream error and the device generation it came from; an error from
    /// a device already replaced is dropped.
    CaptureError(String, u32),
    PlaybackError(String, u32),
    /// Reopens the microphone, the speaker or both (a device change).
    Replace(Option<SourceFactory>, Option<SinkFactory>),
    Stop,
}

#[derive(Default)]
struct Shared {
    closed: AtomicBool,
    packets_dropped: AtomicU64,
    capture_unavailable: AtomicBool,
}

/// Where a capture device delivers audio: mono, 48 kHz, any chunk size.
/// Push-only and allocation-free, so it is safe to call from the device
/// callback. Exactly one callback may use it.
pub struct CaptureTx {
    tx: SyncSender<Event>,
    ring: Arc<Ring>,
    /// A wake-up is already queued; avoids flooding the channel.
    pending: Arc<AtomicBool>,
    shared: Arc<Shared>,
    /// Which opening of the microphone this handle belongs to.
    generation: u32,
}

impl CaptureTx {
    /// Queues a chunk; returns false (dropping what did not fit) when the
    /// ring is full or the pipeline is gone.
    pub fn send(&self, pcm: &[f32]) -> bool {
        if self.shared.closed.load(Ordering::Relaxed) {
            return false;
        }
        let written = self.ring.push_slice(pcm);
        if !self.pending.swap(true, Ordering::AcqRel) {
            match self.tx.try_send(Event::Capture) {
                Ok(()) => {}
                // A later chunk wakes the thread; the data is already queued.
                Err(TrySendError::Full(_)) => self.pending.store(false, Ordering::Release),
                Err(TrySendError::Disconnected(_)) => return false,
            }
        }
        written == pcm.len()
    }

    /// A second handle for the stream's error callback. It shares the queue
    /// and the closed flag, but only [`CaptureTx::report_error`] belongs on
    /// it: the sample ring has a single producer.
    pub(crate) fn clone_for_errors(&self) -> Self {
        self.with_generation(self.generation)
    }

    fn with_generation(&self, generation: u32) -> Self {
        Self {
            tx: self.tx.clone(),
            ring: self.ring.clone(),
            pending: self.pending.clone(),
            shared: self.shared.clone(),
            generation,
        }
    }

    /// Reports a stream failure to the worker (not for the hot path).
    pub fn report_error(&self, message: String) {
        let _ = self
            .tx
            .try_send(Event::CaptureError(message, self.generation));
    }
}

/// How a speaker asks the media thread for more audio. Allocation-free, so
/// it is safe to call from the device callback.
pub struct PlayoutTx {
    tx: SyncSender<Event>,
    pending: Arc<AtomicBool>,
    /// Which opening of the speaker this handle belongs to.
    generation: u32,
}

impl PlayoutTx {
    /// Asks for audio; repeated calls before the thread runs coalesce.
    pub fn request(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) && self.tx.try_send(Event::Playout).is_err() {
            self.pending.store(false, Ordering::Release);
        }
    }

    /// A second handle for the stream's error callback.
    pub(crate) fn clone_for_errors(&self) -> Self {
        self.with_generation(self.generation)
    }

    fn with_generation(&self, generation: u32) -> Self {
        Self {
            tx: self.tx.clone(),
            pending: self.pending.clone(),
            generation,
        }
    }

    /// Reports a stream failure to the worker (not for the hot path).
    pub fn report_error(&self, message: String) {
        let _ = self
            .tx
            .try_send(Event::PlaybackError(message, self.generation));
    }
}

/// A microphone. Started on the media thread, stopped by dropping.
pub trait AudioSource {
    fn start(&mut self, out: CaptureTx) -> Result<(), PipelineError>;
}

/// A speaker. It owns the playout clock: when its queue runs low it calls
/// [`PlayoutTx::request`], and the media thread answers with `write`.
pub trait AudioSink {
    /// Starts the device; `demand` is how it asks for audio.
    fn start(&mut self, demand: PlayoutTx) -> Result<(), PipelineError>;
    /// Queues mono 48 kHz PCM; must not block.
    fn write(&mut self, pcm: &[f32]);
    /// Audio queued for the device, in 48 kHz samples.
    fn queued_samples(&self) -> usize;
    /// Estimated delay from `write` to the speaker, in milliseconds: the
    /// queue plus the device latency.
    fn delay_ms(&self) -> u32;
}

/// Devices are opened on the media thread (some backends are `!Send`).
pub type SourceFactory = Box<dyn FnOnce() -> Result<Box<dyn AudioSource>, PipelineError> + Send>;
pub type SinkFactory = Box<dyn FnOnce() -> Result<Box<dyn AudioSink>, PipelineError> + Send>;

/// Handle to the running pipeline.
pub struct MediaPipeline {
    tx: SyncSender<Event>,
    muted: Arc<AtomicBool>,
    shared: Arc<Shared>,
    ring: Arc<Ring>,
    /// Bumped by [`MediaPipeline::reset_stream`]; packets carry it.
    generation: Arc<AtomicU32>,
    /// Signalled by the media thread as it exits.
    exited: Option<mpsc::Receiver<()>>,
    thread: Option<JoinHandle<()>>,
}

/// A pipeline whose devices are still opening. Dropping it abandons the
/// start without waiting: the media thread is detached and stops itself as
/// soon as (if ever) the device calls return.
pub struct PendingPipeline {
    pipeline: Option<MediaPipeline>,
    ready: tokio::sync::oneshot::Receiver<Result<(), PipelineError>>,
}

impl PendingPipeline {
    /// Resolves when the devices run or failed. Cancel-safe: awaiting it
    /// again after a cancel continues to wait.
    pub async fn wait(&mut self) -> Result<(), PipelineError> {
        match (&mut self.ready).await {
            Ok(r) => r,
            Err(_) => Err(PipelineError::Thread("media thread exited early".into())),
        }
    }

    /// Joins the thread of a start that failed (it is already exiting).
    fn join_exited(&mut self) {
        if let Some(thread) = self.pipeline.as_mut().and_then(|p| p.thread.take()) {
            let _ = thread.join();
        }
    }

    /// The running pipeline, after [`PendingPipeline::wait`] returned `Ok`.
    pub fn into_pipeline(mut self) -> Option<MediaPipeline> {
        self.pipeline.take()
    }
}

impl Drop for PendingPipeline {
    fn drop(&mut self) {
        if let Some(mut p) = self.pipeline.take() {
            // Never join a thread that may be stuck inside a device call.
            p.shared.closed.store(true, Ordering::Relaxed);
            let _ = p.tx.try_send(Event::Stop);
            drop(p.thread.take());
        }
    }
}

impl MediaPipeline {
    /// Spawns the media thread, opens the devices on it and returns once
    /// they run (or failed). A missing or failing microphone does not fail
    /// the start: the call runs receive-only and `events` gets
    /// [`PipelineEvent::CaptureUnavailable`]. A speaker failure does.
    /// Blocks until the devices answer; async callers use
    /// [`MediaPipeline::begin`].
    pub fn start(
        source: SourceFactory,
        sink: SinkFactory,
        out: PacketOut,
        events: EventOut,
    ) -> Result<Self, PipelineError> {
        let mut pending = Self::begin(source, sink, out, events)?;
        match futures::executor::block_on(pending.wait()) {
            Ok(()) => pending
                .into_pipeline()
                .ok_or_else(|| PipelineError::Thread("pipeline missing".into())),
            Err(e) => {
                pending.join_exited();
                Err(e)
            }
        }
    }

    /// Spawns the media thread and returns at once; the devices open on that
    /// thread and [`PendingPipeline::wait`] reports the outcome.
    pub fn begin(
        source: SourceFactory,
        sink: SinkFactory,
        out: PacketOut,
        events: EventOut,
    ) -> Result<PendingPipeline, PipelineError> {
        let (tx, rx) = mpsc::sync_channel::<Event>(QUEUE);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<(), PipelineError>>();
        let (exit_tx, exit_rx) = mpsc::sync_channel::<()>(1);
        let muted = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared::default());
        let ring = Arc::new(Ring::new(CAPTURE_RING));
        let capture_pending = Arc::new(AtomicBool::new(false));
        let setup = Setup {
            source,
            sink,
            out,
            events,
            muted: muted.clone(),
            capture: CaptureTx {
                tx: tx.clone(),
                ring: ring.clone(),
                pending: capture_pending.clone(),
                shared: shared.clone(),
                generation: 0,
            },
            playout: PlayoutTx {
                tx: tx.clone(),
                pending: Arc::new(AtomicBool::new(false)),
                generation: 0,
            },
            ring: ring.clone(),
            capture_pending,
        };
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("parla-media".into())
            .spawn(move || {
                match Worker::open(setup) {
                    Ok(mut worker) => {
                        // Abandoned while opening: do not start running.
                        if ready_tx.send(Ok(())).is_ok() {
                            worker.run(&rx);
                        }
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
                thread_shared.closed.store(true, Ordering::Relaxed);
                let _ = exit_tx.try_send(());
            })
            .map_err(|e| PipelineError::Thread(e.to_string()))?;
        Ok(PendingPipeline {
            pipeline: Some(Self {
                tx,
                muted,
                shared,
                ring,
                generation: Arc::new(AtomicU32::new(0)),
                exited: Some(exit_rx),
                thread: Some(thread),
            }),
            ready: ready_rx,
        })
    }

    /// Hands a received packet to the jitter buffer; dropped (and counted)
    /// when the queue is full.
    pub fn push_packet(&self, packet: RtpPacket) {
        let generation = self.generation.load(Ordering::Relaxed);
        if self.tx.try_send(Event::Packet(packet, generation)).is_err() {
            self.shared.packets_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The remote stream changed (a new SSRC): drops buffered packets and the
    /// sequence state so the new stream is not judged against the old one.
    /// Never blocks: it bumps a generation that later packets carry, so the
    /// reset applies exactly between the packets pushed before and after it,
    /// whatever the queue holds.
    pub fn reset_stream(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Reopens the microphone (`source`), the speaker (`sink`) or both on the
    /// media thread, for a device change or after a stream error. The old
    /// device closes first; a microphone that does not open leaves the call
    /// receive-only ([`PipelineEvent::CaptureUnavailable`]), a speaker that
    /// does not start keeps the old one and reports
    /// [`PipelineEvent::PlaybackError`]. Never blocks: when the queue is full
    /// the factories come back as the error, for the caller to retry.
    #[allow(clippy::type_complexity)]
    pub fn replace_devices(
        &self,
        source: Option<SourceFactory>,
        sink: Option<SinkFactory>,
    ) -> Result<(), (Option<SourceFactory>, Option<SinkFactory>)> {
        if source.is_none() && sink.is_none() {
            return Ok(());
        }
        match self.tx.try_send(Event::Replace(source, sink)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(Event::Replace(a, b))) => Err((a, b)),
            // The media thread is gone; nothing to reopen.
            Err(_) => Ok(()),
        }
    }

    /// Capture gate: while muted nothing is encoded or sent. Capture stays
    /// open and keeps feeding the echo canceller (it needs the microphone
    /// signal to track the echo path), and playback keeps running; only the
    /// encode and send step is skipped.
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn stats(&self) -> PipelineStats {
        PipelineStats {
            capture_dropped: self.ring.dropped(),
            packets_dropped: self.shared.packets_dropped.load(Ordering::Relaxed),
            capture_unavailable: self.shared.capture_unavailable.load(Ordering::Relaxed),
        }
    }

    /// Stops the thread, closes the devices and joins (at most
    /// [`STOP_WITHIN`], then the thread is detached).
    pub fn stop(mut self) {
        self.shutdown(STOP_WITHIN);
    }

    /// Like [`MediaPipeline::stop`] with a caller-chosen limit. Returns false
    /// when the thread did not exit in time (a device call is stuck) and was
    /// detached.
    pub fn stop_within(mut self, limit: Duration) -> bool {
        self.shutdown(limit)
    }

    fn shutdown(&mut self, limit: Duration) -> bool {
        let Some(thread) = self.thread.take() else {
            return true;
        };
        self.shared.closed.store(true, Ordering::Relaxed);
        // A full queue means the thread is busy and drains it; it also sees
        // `closed` after each event, so a lost `Stop` is not needed.
        let _ = self.tx.try_send(Event::Stop);
        let exited = match self.exited.take() {
            Some(rx) => !matches!(rx.recv_timeout(limit), Err(mpsc::RecvTimeoutError::Timeout)),
            None => true,
        };
        if exited {
            let _ = thread.join();
        } else {
            tracing::warn!("media thread did not stop within {limit:?}");
        }
        exited
    }
}

impl Drop for MediaPipeline {
    fn drop(&mut self) {
        self.shutdown(STOP_WITHIN);
    }
}

struct Setup {
    source: SourceFactory,
    sink: SinkFactory,
    out: PacketOut,
    events: EventOut,
    muted: Arc<AtomicBool>,
    capture: CaptureTx,
    playout: PlayoutTx,
    ring: Arc<Ring>,
    capture_pending: Arc<AtomicBool>,
}

struct Worker {
    // Held for its Drop: closing the microphone. None when it never opened.
    source: Option<Box<dyn AudioSource>>,
    sink: Box<dyn AudioSink>,
    /// Templates for a reopened microphone and speaker.
    capture: CaptureTx,
    playout: PlayoutTx,
    out: PacketOut,
    events: EventOut,
    muted: Arc<AtomicBool>,
    ring: Arc<Ring>,
    capture_pending: Arc<AtomicBool>,
    playout_pending: Arc<AtomicBool>,
    apm: EchoProcessor,
    encoder: FrameEncoder,
    decoder: FrameDecoder,
    jitter: JitterBuffer,
    /// The capture frame being assembled from the ring.
    capframe: Vec<f32>,
    capfill: usize,
    /// Decode output; holds the longest Opus packet.
    decoded: Vec<f32>,
    timestamp: u32,
    /// Stream generation of the last packet pushed.
    generation: u32,
    shared: Arc<Shared>,
}

impl Worker {
    fn open(setup: Setup) -> Result<Self, PipelineError> {
        let Setup {
            source,
            sink,
            out,
            mut events,
            muted,
            capture,
            playout,
            ring,
            capture_pending,
        } = setup;
        let encoder = FrameEncoder::new()?;
        let decoder = FrameDecoder::new()?;
        let playout_pending = playout.pending.clone();
        let shared = capture.shared.clone();
        let capture_template = capture.clone_for_errors();
        let playout_template = playout.clone_for_errors();
        let mut sink = sink()?;
        sink.start(playout)?;
        // A missing microphone must not take the call down: fall back to no
        // capture and say so.
        let source = match source().and_then(|mut s| s.start(capture).map(|()| s)) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!("no microphone, receiving only: {e}");
                shared.capture_unavailable.store(true, Ordering::Relaxed);
                events(PipelineEvent::CaptureUnavailable(e.to_string()));
                None
            }
        };
        Ok(Self {
            source,
            sink,
            capture: capture_template,
            playout: playout_template,
            out,
            events,
            muted,
            ring,
            capture_pending,
            playout_pending,
            apm: EchoProcessor::new(),
            encoder,
            decoder,
            jitter: JitterBuffer::new(),
            capframe: vec![0.0; FRAME_SAMPLES],
            capfill: 0,
            decoded: vec![0.0; MAX_DECODE_SAMPLES],
            timestamp: 0,
            generation: 0,
            shared,
        })
    }

    fn run(&mut self, rx: &mpsc::Receiver<Event>) {
        while let Ok(event) = rx.recv() {
            match event {
                Event::Capture => {
                    // Clear first: data pushed after this point wakes us again.
                    self.capture_pending.store(false, Ordering::Release);
                    self.on_capture();
                }
                Event::Playout => {
                    self.playout_pending.store(false, Ordering::Release);
                    self.on_playout();
                }
                Event::Packet(p, generation) => {
                    if generation != self.generation {
                        self.generation = generation;
                        self.jitter.reset();
                    }
                    self.jitter.push(p.seq, p.payload);
                }
                Event::CaptureError(e, g) if g == self.capture.generation => {
                    tracing::warn!("capture stream error: {e}");
                    (self.events)(PipelineEvent::CaptureError(e));
                }
                Event::PlaybackError(e, g) if g == self.playout.generation => {
                    tracing::warn!("playback stream error: {e}");
                    (self.events)(PipelineEvent::PlaybackError(e));
                }
                // From a device already replaced.
                Event::CaptureError(..) | Event::PlaybackError(..) => {}
                Event::Replace(source, sink) => {
                    if let Some(f) = sink {
                        self.reopen_sink(f);
                    }
                    if let Some(f) = source {
                        self.reopen_source(f);
                    }
                }
                Event::Stop => break,
            }
            if self.shared.closed.load(Ordering::Relaxed) {
                break;
            }
        }
    }

    /// Opens the new speaker before closing the old one, so a failure keeps
    /// the old one playing.
    fn reopen_sink(&mut self, factory: SinkFactory) {
        let next = self
            .playout
            .with_generation(self.playout.generation.wrapping_add(1));
        let opened = factory().and_then(|mut sink| {
            sink.start(next.clone_for_errors())?;
            Ok(sink)
        });
        match opened {
            Ok(sink) => {
                // The old speaker closes here; its late errors are dropped.
                self.sink = sink;
                self.playout = next;
                self.playout_pending.store(false, Ordering::Release);
                self.playout.request();
            }
            Err(e) => {
                tracing::warn!("speaker did not reopen: {e}");
                (self.events)(PipelineEvent::PlaybackError(e.to_string()));
            }
        }
    }

    /// Closes the old microphone (the capture ring has one producer), then
    /// starts the new one; without one the call goes on receive-only.
    fn reopen_source(&mut self, factory: SourceFactory) {
        self.source = None;
        // A new generation even on failure: the old stream's errors are stale.
        self.capture = self
            .capture
            .with_generation(self.capture.generation.wrapping_add(1));
        match factory().and_then(|mut s| s.start(self.capture.clone_for_errors()).map(|()| s)) {
            Ok(s) => {
                self.source = Some(s);
                self.shared
                    .capture_unavailable
                    .store(false, Ordering::Relaxed);
            }
            Err(e) => {
                tracing::warn!("microphone did not reopen, receiving only: {e}");
                self.shared
                    .capture_unavailable
                    .store(true, Ordering::Relaxed);
                (self.events)(PipelineEvent::CaptureUnavailable(e.to_string()));
            }
        }
    }

    /// Cleans and encodes every full 20 ms frame waiting in the ring.
    fn on_capture(&mut self) {
        self.apm.set_stream_delay_ms(self.sink.delay_ms());
        let mut frame = std::mem::take(&mut self.capframe);
        loop {
            self.capfill += self.ring.pop_slice(&mut frame[self.capfill..]);
            if self.capfill < FRAME_SAMPLES {
                break;
            }
            self.capfill = 0;
            self.encode_frame(&mut frame);
        }
        self.capframe = frame;
    }

    fn encode_frame(&mut self, captured: &mut [f32]) {
        for block in captured.chunks_mut(APM_SAMPLES) {
            self.apm.capture(block);
        }
        let timestamp = self.timestamp;
        self.timestamp = self.timestamp.wrapping_add(FRAME_SAMPLES as u32);
        if self.muted.load(Ordering::Relaxed) {
            return;
        }
        match self.encoder.encode(captured) {
            Ok(payload) => (self.out)(EncodedFrame { timestamp, payload }),
            Err(e) => tracing::warn!("opus encode failed: {e}"),
        }
    }

    /// Tops the speaker queue up to its target from the jitter buffer.
    fn on_playout(&mut self) {
        for _ in 0..MAX_FRAMES_PER_WAKE {
            if self.sink.queued_samples() >= SINK_TARGET_SAMPLES || !self.play_one() {
                break;
            }
        }
    }

    /// Decodes and plays one 20 ms frame; false when there is nothing to
    /// play yet (the jitter buffer is still filling).
    fn play_one(&mut self) -> bool {
        let decoded = match self.jitter.pop() {
            Playout::Frame(payload) => self.decoder.decode(&payload, &mut self.decoded),
            Playout::Lost => match self.jitter.next_payload() {
                // The next packet carries the lost frame's FEC.
                Some(next) => self.decoder.decode_fec(next, &mut self.decoded),
                None => self.decoder.decode(&[], &mut self.decoded),
            },
            Playout::Empty => return false,
        };
        let frame = &mut self.decoded[..FRAME_SAMPLES];
        match decoded {
            Ok(n) => frame[n.min(FRAME_SAMPLES)..].fill(0.0),
            Err(e) => {
                tracing::warn!("opus decode failed: {e}");
                // Keep the echo canceller's far end and the sink in step.
                frame.fill(0.0);
            }
        }
        for block in frame.chunks(APM_SAMPLES) {
            self.apm.render(block);
        }
        self.sink.write(frame);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    const WAIT: Duration = Duration::from_secs(2);

    struct TestSource(mpsc::Sender<CaptureTx>);
    impl AudioSource for TestSource {
        fn start(&mut self, out: CaptureTx) -> Result<(), PipelineError> {
            self.0.send(out).unwrap();
            Ok(())
        }
    }

    /// A speaker whose queue the test drains by hand.
    struct TestSink {
        played: Arc<Mutex<Vec<f32>>>,
        level: Arc<AtomicUsize>,
        demand: mpsc::Sender<PlayoutTx>,
        wrote: mpsc::Sender<()>,
    }
    impl AudioSink for TestSink {
        fn start(&mut self, demand: PlayoutTx) -> Result<(), PipelineError> {
            self.demand.send(demand).unwrap();
            Ok(())
        }
        fn write(&mut self, pcm: &[f32]) {
            self.played.lock().unwrap().extend_from_slice(pcm);
            self.level.fetch_add(pcm.len(), Ordering::Relaxed);
            let _ = self.wrote.send(());
        }
        fn queued_samples(&self) -> usize {
            self.level.load(Ordering::Relaxed)
        }
        fn delay_ms(&self) -> u32 {
            20
        }
    }

    struct Rig {
        pipeline: MediaPipeline,
        capture: Option<CaptureTx>,
        demand: PlayoutTx,
        level: Arc<AtomicUsize>,
        played: Arc<Mutex<Vec<f32>>>,
        wrote: mpsc::Receiver<()>,
        packets: mpsc::Receiver<EncodedFrame>,
        events: mpsc::Receiver<PipelineEvent>,
    }

    fn rig_with(source: SourceFactory, capture_rx: Option<mpsc::Receiver<CaptureTx>>) -> Rig {
        let played = Arc::new(Mutex::new(Vec::new()));
        let level = Arc::new(AtomicUsize::new(0));
        let (dtx, drx) = mpsc::channel();
        let (wtx, wrote) = mpsc::channel();
        let sink = TestSink {
            played: played.clone(),
            level: level.clone(),
            demand: dtx,
            wrote: wtx,
        };
        let (ptx, packets) = mpsc::channel();
        let (etx, events) = mpsc::channel();
        let pipeline = MediaPipeline::start(
            source,
            Box::new(move || Ok(Box::new(sink) as Box<dyn AudioSink>)),
            Box::new(move |f| {
                let _ = ptx.send(f);
            }),
            Box::new(move |e| {
                let _ = etx.send(e);
            }),
        )
        .unwrap();
        Rig {
            pipeline,
            capture: capture_rx.map(|rx| rx.recv_timeout(WAIT).unwrap()),
            demand: drx.recv_timeout(WAIT).unwrap(),
            level,
            played,
            wrote,
            packets,
            events,
        }
    }

    fn rig() -> Rig {
        let (ctx, crx) = mpsc::channel();
        rig_with(
            Box::new(move || Ok(Box::new(TestSource(ctx)) as Box<dyn AudioSource>)),
            Some(crx),
        )
    }

    impl Rig {
        fn capture(&self) -> &CaptureTx {
            self.capture.as_ref().unwrap()
        }

        /// The device drains its queue and asks for audio; waits for the
        /// frames the media thread answers with.
        fn play_round(&self, frames: usize) {
            self.level.store(0, Ordering::Relaxed);
            self.demand.request();
            for _ in 0..frames {
                self.wrote.recv_timeout(WAIT).unwrap();
            }
        }
    }

    fn tone_chunk(start: usize, n: usize) -> Vec<f32> {
        (start..start + n)
            .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin())
            .collect()
    }

    /// Feeds one 20 ms frame at a time, waiting for its packet so the
    /// capture ring never overflows.
    fn feed_frames(rig: &Rig, from: usize, count: usize) -> Vec<EncodedFrame> {
        let mut out = Vec::new();
        for f in 0..count {
            assert!(
                rig.capture()
                    .send(&tone_chunk((from + f) * FRAME_SAMPLES, FRAME_SAMPLES))
            );
            out.push(rig.packets.recv_timeout(WAIT).unwrap());
        }
        out
    }

    fn dominant_hz(pcm: &[f32]) -> f32 {
        let zc = pcm.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        zc as f32 / (pcm.len() as f32 / 48_000.0)
    }

    /// 440 Hz Opus packets, as a remote peer would send them.
    fn remote_packets(count: usize) -> Vec<Vec<u8>> {
        let mut enc = FrameEncoder::new().unwrap();
        (0..count)
            .map(|f| {
                enc.encode(&tone_chunk(f * FRAME_SAMPLES, FRAME_SAMPLES))
                    .unwrap()
            })
            .collect()
    }

    fn push_one(rig: &Rig, seq: usize, payload: &[u8]) {
        rig.pipeline.push_packet(RtpPacket {
            seq: seq as u16,
            payload: payload.to_vec(),
        });
    }

    /// Plays the packets the way a speaker would consume them: two frames
    /// per round, with the network keeping the jitter buffer fed.
    fn play_stream(rig: &Rig, packets: &[Vec<u8>]) {
        for (i, p) in packets.iter().enumerate().take(4) {
            push_one(rig, i, p);
        }
        for round in 0..(packets.len() - 4) / 2 {
            rig.play_round(2);
            for (i, p) in packets.iter().enumerate().skip(4 + round * 2).take(2) {
                push_one(rig, i, p);
            }
        }
    }

    fn assert_440hz(rig: &Rig) {
        let played = rig.played.lock().unwrap().clone();
        assert!(played.len() >= 20 * FRAME_SAMPLES, "{}", played.len());
        let tail = &played[played.len() - 8 * FRAME_SAMPLES..];
        let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
        let hz = dominant_hz(tail);
        assert!((hz - 440.0).abs() < 20.0, "{hz} rms {rms}");
        assert!(rms > 0.1, "rms {rms}");
    }

    #[test]
    fn capture_to_packets_to_playback_keeps_the_tone() {
        let rig = rig();
        let sent = feed_frames(&rig, 0, 30);
        assert_eq!(sent[1].timestamp - sent[0].timestamp, FRAME_SAMPLES as u32);
        let payloads: Vec<Vec<u8>> = sent.iter().map(|f| f.payload.clone()).collect();
        play_stream(&rig, &payloads);
        assert_440hz(&rig);
        rig.pipeline.stop();
    }

    #[test]
    fn reset_stream_lets_a_restarted_sequence_play() {
        let rig = rig();
        let packets = remote_packets(30);
        play_stream(&rig, &packets);
        rig.played.lock().unwrap().clear();
        // A new SSRC restarts the sequence far from where the old one ended.
        rig.pipeline.reset_stream();
        for (i, p) in packets.iter().enumerate().take(4) {
            push_one(&rig, 40_000 + i, p);
        }
        for round in 0..(packets.len() - 4) / 2 {
            rig.play_round(2);
            for (i, p) in packets.iter().enumerate().skip(4 + round * 2).take(2) {
                push_one(&rig, 40_000 + i, p);
            }
        }
        assert_440hz(&rig);
        rig.pipeline.stop();
    }

    #[test]
    fn playout_is_driven_by_the_speaker_not_by_capture() {
        // A microphone that opens but never delivers a sample.
        let (ctx, crx) = mpsc::channel();
        let rig = rig_with(
            Box::new(move || Ok(Box::new(TestSource(ctx)) as Box<dyn AudioSource>)),
            Some(crx),
        );
        play_stream(&rig, &remote_packets(30));
        assert_440hz(&rig);
        assert!(rig.packets.try_recv().is_err());
        rig.pipeline.stop();
    }

    #[test]
    fn no_microphone_still_plays_remote_audio() {
        let rig = rig_with(
            Box::new(|| Err(PipelineError::Device("no input device".into()))),
            None,
        );
        assert_eq!(
            rig.events.recv_timeout(WAIT).unwrap(),
            PipelineEvent::CaptureUnavailable("audio device: no input device".into())
        );
        assert!(rig.pipeline.stats().capture_unavailable);
        play_stream(&rig, &remote_packets(30));
        assert_440hz(&rig);
        rig.pipeline.stop();
    }

    #[test]
    fn microphone_that_fails_to_start_falls_back_too() {
        struct Broken;
        impl AudioSource for Broken {
            fn start(&mut self, _: CaptureTx) -> Result<(), PipelineError> {
                Err(PipelineError::Device("busy".into()))
            }
        }
        let rig = rig_with(
            Box::new(|| Ok(Box::new(Broken) as Box<dyn AudioSource>)),
            None,
        );
        assert!(matches!(
            rig.events.recv_timeout(WAIT).unwrap(),
            PipelineEvent::CaptureUnavailable(_)
        ));
        rig.pipeline.stop();
    }

    #[test]
    fn capture_errors_reach_the_worker_and_playout_continues() {
        let rig = rig();
        rig.capture().report_error("device unplugged".into());
        assert_eq!(
            rig.events.recv_timeout(WAIT).unwrap(),
            PipelineEvent::CaptureError("device unplugged".into())
        );
        play_stream(&rig, &remote_packets(30));
        assert_440hz(&rig);
        rig.pipeline.stop();
    }

    #[test]
    fn begin_returns_at_once_when_a_device_hangs() {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let t = Instant::now();
        let pending = MediaPipeline::begin(
            Box::new(|| Err(PipelineError::Device("no mic".into()))),
            Box::new(move || {
                let _ = release_rx.recv_timeout(Duration::from_secs(30));
                Err(PipelineError::Device("late".into()))
            }),
            Box::new(|_| {}),
            Box::new(|_| {}),
        )
        .unwrap();
        assert!(t.elapsed() < Duration::from_millis(500));
        // Abandoning the start neither blocks nor joins the stuck thread.
        drop(pending);
        assert!(t.elapsed() < Duration::from_millis(500));
        drop(release_tx);
    }

    #[test]
    fn reset_stream_never_waits_for_queue_room() {
        let rig = rig();
        let t = Instant::now();
        for _ in 0..10_000 {
            rig.pipeline.reset_stream();
        }
        assert!(t.elapsed() < Duration::from_secs(1));
        rig.pipeline.stop();
    }

    #[test]
    fn stop_within_gives_up_on_a_stuck_media_thread() {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (entered_tx, entered) = mpsc::channel::<()>();
        let sink = TestSinkStuck {
            entered: entered_tx,
            release: Some(release_rx),
        };
        let pipeline = MediaPipeline::start(
            Box::new(|| Err(PipelineError::Device("no mic".into()))),
            Box::new(move || Ok(Box::new(sink) as Box<dyn AudioSink>)),
            Box::new(|_| {}),
            Box::new(|_| {}),
        )
        .unwrap();
        let _ = entered; // the sink only blocks once it is dropped
        let t = Instant::now();
        let joined = pipeline.stop_within(Duration::from_millis(200));
        assert!(!joined);
        assert!(t.elapsed() < Duration::from_secs(1));
        drop(release_tx);
    }

    /// A speaker whose drop (closing the device) hangs until released.
    struct TestSinkStuck {
        entered: mpsc::Sender<()>,
        release: Option<mpsc::Receiver<()>>,
    }
    impl AudioSink for TestSinkStuck {
        fn start(&mut self, _demand: PlayoutTx) -> Result<(), PipelineError> {
            Ok(())
        }
        fn write(&mut self, _pcm: &[f32]) {}
        fn queued_samples(&self) -> usize {
            0
        }
        fn delay_ms(&self) -> u32 {
            20
        }
    }
    impl Drop for TestSinkStuck {
        fn drop(&mut self) {
            let _ = self.entered.send(());
            if let Some(r) = self.release.take() {
                let _ = r.recv_timeout(Duration::from_secs(30));
            }
        }
    }

    #[test]
    fn a_speaker_failure_fails_the_start_and_joins() {
        let r = MediaPipeline::start(
            Box::new(|| Err(PipelineError::Device("no mic".into()))),
            Box::new(|| Err(PipelineError::Device("no speaker".into()))),
            Box::new(|_| {}),
            Box::new(|_| {}),
        );
        assert!(matches!(r, Err(PipelineError::Device(_))));
    }

    #[test]
    fn a_full_speaker_queue_asks_for_nothing() {
        let rig = rig();
        for (i, p) in remote_packets(6).iter().enumerate() {
            push_one(&rig, i, p);
        }
        rig.level.store(SINK_TARGET_SAMPLES, Ordering::Relaxed);
        rig.demand.request();
        let Rig {
            pipeline, played, ..
        } = rig;
        pipeline.stop();
        assert!(played.lock().unwrap().is_empty());
    }

    #[test]
    fn mute_gate_stops_packets_and_resumes() {
        let rig = rig();
        feed_frames(&rig, 0, 3);
        rig.pipeline.set_muted(true);
        assert!(rig.pipeline.is_muted());
        for f in 3..8 {
            rig.capture()
                .send(&tone_chunk(f * FRAME_SAMPLES, FRAME_SAMPLES));
        }
        let Rig {
            pipeline, packets, ..
        } = rig;
        // Joining drains the queue, so every muted frame has been seen.
        pipeline.stop();
        assert!(packets.try_recv().is_err(), "packet sent while muted");
    }

    #[test]
    fn stop_joins_within_two_seconds() {
        let rig = rig();
        feed_frames(&rig, 0, 2);
        let t = Instant::now();
        rig.pipeline.stop();
        assert!(t.elapsed() < WAIT);
    }

    #[test]
    fn dropping_the_handle_stops_the_thread() {
        let mut rig = rig();
        let capture = rig.capture.take().unwrap();
        drop(rig);
        // Pipeline gone: the device callback learns it ended.
        assert!(!capture.send(&[0.0; 10]));
    }

    #[test]
    fn full_capture_ring_drops_and_counts_instead_of_growing() {
        let (tx, _rx) = mpsc::sync_channel(2);
        let shared = Arc::new(Shared::default());
        let capture = CaptureTx {
            tx,
            ring: Arc::new(Ring::new(8)),
            pending: Arc::new(AtomicBool::new(false)),
            shared,
            generation: 0,
        };
        assert!(capture.send(&[0.0; 8]));
        assert!(!capture.send(&[0.0; 3]));
        assert_eq!(capture.ring.dropped(), 3);
    }

    #[test]
    fn replace_devices_reopens_capture_and_playback() {
        let rig = rig();
        // A new microphone and speaker, as after a device change.
        let (ctx, crx) = mpsc::channel();
        let played = Arc::new(Mutex::new(Vec::new()));
        let level = Arc::new(AtomicUsize::new(0));
        let (dtx, drx) = mpsc::channel();
        let (wtx, wrote) = mpsc::channel();
        let sink = TestSink {
            played: played.clone(),
            level: level.clone(),
            demand: dtx,
            wrote: wtx,
        };
        let sent = rig.pipeline.replace_devices(
            Some(Box::new(move || {
                Ok(Box::new(TestSource(ctx)) as Box<dyn AudioSource>)
            })),
            Some(Box::new(move || Ok(Box::new(sink) as Box<dyn AudioSink>))),
        );
        assert!(sent.is_ok());
        let capture = crx.recv_timeout(WAIT).expect("the new microphone starts");
        let demand = drx.recv_timeout(WAIT).expect("the new speaker starts");
        // Capture from the new microphone is encoded and sent.
        assert!(capture.send(&tone_chunk(0, FRAME_SAMPLES)));
        rig.packets
            .recv_timeout(WAIT)
            .expect("a frame from the new microphone");
        // Received audio plays on the new speaker, not the old one.
        let before = rig.played.lock().unwrap().len();
        for (i, p) in remote_packets(6).iter().enumerate() {
            push_one(&rig, i, p);
        }
        demand.request();
        wrote.recv_timeout(WAIT).expect("the new speaker plays");
        assert!(!played.lock().unwrap().is_empty());
        assert_eq!(rig.played.lock().unwrap().len(), before);
    }

    #[test]
    fn a_failed_reopen_reports_capture_unavailable() {
        let rig = rig();
        let sent = rig.pipeline.replace_devices(
            Some(Box::new(|| Err(PipelineError::Device("gone".into())))),
            None,
        );
        assert!(sent.is_ok());
        let e = rig.events.recv_timeout(WAIT).unwrap();
        assert!(matches!(e, PipelineEvent::CaptureUnavailable(_)), "{e:?}");
    }

    #[test]
    fn a_playback_stream_error_is_reported() {
        let rig = rig();
        rig.demand.report_error("device unplugged".into());
        let e = rig.events.recv_timeout(WAIT).unwrap();
        assert_eq!(e, PipelineEvent::PlaybackError("device unplugged".into()));
    }

    #[test]
    fn errors_from_a_replaced_stream_are_dropped() {
        let rig = rig();
        let (ctx, crx) = mpsc::channel();
        let sent = rig.pipeline.replace_devices(
            Some(Box::new(move || {
                Ok(Box::new(TestSource(ctx)) as Box<dyn AudioSource>)
            })),
            None,
        );
        assert!(sent.is_ok());
        let fresh = crx.recv_timeout(WAIT).unwrap();
        // The old microphone's late error says nothing about the new one.
        rig.capture().report_error("old stream".into());
        fresh.report_error("new stream".into());
        let e = rig.events.recv_timeout(WAIT).unwrap();
        assert_eq!(e, PipelineEvent::CaptureError("new stream".into()));
        assert!(rig.events.recv_timeout(Duration::from_millis(100)).is_err());
    }
}
