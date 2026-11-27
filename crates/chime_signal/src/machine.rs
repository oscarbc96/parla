//! Sans-IO signalling state machine for a native huddle join.
//!
//! Nothing here opens a socket, spawns a thread or arms a timer. The caller
//! feeds [`Input`]s with the current `Instant`, writes every [`Output::Send`]
//! to the websocket and sleeps until [`Machine::next_deadline`] when it has
//! nothing else to wait for.

use crate::codec::{decode, encode};
use crate::proto::sdk_signal_frame::Type;
use crate::proto::*;
use crate::secret::Secret;
use std::time::{Duration, Instant};

/// How long the join ack, the index and the subscribe ack may each take.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for a leave ack or the socket close before giving up.
pub const LEAVE_TIMEOUT: Duration = Duration::from_secs(2);
/// Client keepalive ping interval once the socket is open.
pub const PING_INTERVAL: Duration = Duration::from_secs(10);
/// Audio metadata reports each stream's level as an attenuation in dB
/// (0 loudest, 127 silent). Like the Chime JS SDK's volume indicator, a level
/// above -42 dB counts as sound; this cut-off leaves a little headroom over
/// that floor so line noise does not read as speech. A silent stream starts
/// speaking at or below this attenuation.
pub const SPEAKING_MAX_ATTENUATION: u32 = 39;
/// A speaking stream counts as quiet only at or above this attenuation; the
/// band in between keeps whatever state the stream had (hysteresis).
pub const QUIET_MIN_ATTENUATION: u32 = 46;
/// A speaking stream that went quiet stays speaking this long after its last
/// loud level, so pauses between words do not flicker. Measured on frame
/// arrival times: no timer runs for it.
pub const SPEAKING_HOLD: Duration = Duration::from_millis(800);
/// Streams whose speaking state is kept; more are ignored (a huddle has far
/// fewer attendees).
pub const MAX_TRACKED_STREAMS: usize = 256;

/// Who the client says it is in the join frame.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub app_name: String,
    pub app_version: String,
    pub platform_name: String,
    pub os_name: String,
    pub os_version: String,
    /// Audio host the media will check in with (from the join credentials).
    pub audio_host: String,
    /// Audio session id for the join frame; `None` picks a random one.
    pub audio_session_id: Option<u32>,
    /// Our own attendee id (from the join input); labels the declared send stream.
    pub attendee_id: String,
}

/// TURN credentials from the join ack.
#[derive(Clone, PartialEq, Eq)]
pub struct TurnCredentials {
    pub username: Secret,
    pub password: Secret,
    pub ttl_seconds: u32,
    pub uris: Vec<String>,
}

impl std::fmt::Debug for TurnCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnCredentials")
            .field("username", &"[redacted]")
            .field("password", &"[redacted]")
            .field("ttl_seconds", &self.ttl_seconds)
            .field("uris", &self.uris)
            .finish()
    }
}

/// One attendee as the audio stream id info frame reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Participant {
    pub attendee_id: String,
    pub external_user_id: String,
    pub muted: bool,
    pub dropped: bool,
    pub stream_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Opened,
    Frame(Vec<u8>),
    Tick,
    LocalOffer {
        sdp: String,
    },
    SetMuted(bool),
    Leave,
    /// The socket closed abnormally or without a close frame.
    Closed,
    /// The server closed the socket with a normal close code: a clean end
    /// once joined.
    ClosedNormally,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    TurnCredentials(TurnCredentials),
    RemoteAnswer {
        sdp: String,
    },
    Participants(Vec<Participant>),
    Index {
        num_participants: u32,
    },
    /// The audio stream ids speaking now, ascending. Sent only when the set
    /// changes, never per metadata frame.
    Speaking(Vec<u32>),
    Error {
        status: u32,
        description: String,
    },
    Left,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    Send(Vec<u8>),
    Event(Event),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Created,
    AwaitJoinAck,
    AwaitIndex,
    AwaitOffer,
    AwaitSubscribeAck,
    Joined,
    Leaving,
    Left,
    Failed,
}

pub struct Machine {
    config: Config,
    state: State,
    audio_session_id: u32,
    epoch_ms: u64,
    started: Instant,
    /// Deadline of the current handshake or leave step.
    deadline: Option<Instant>,
    next_ping: Option<Instant>,
    ping_id: u32,
    muted: bool,
    /// The mute value the subscribe frame carried.
    subscribed_muted: bool,
    offer: Option<String>,
    /// Level state per stream, from audio metadata; bounded by
    /// [`MAX_TRACKED_STREAMS`].
    levels: std::collections::BTreeMap<u32, Level>,
    /// Streams speaking as last reported.
    speaking: Vec<u32>,
}

/// One stream's level state.
#[derive(Debug, Clone, Copy)]
struct Level {
    attenuation: u32,
    muted: bool,
    speaking: bool,
    /// When it was last loud.
    loud_at: Instant,
}

fn random_session_id() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    // `RandomState` is seeded by the OS per instance: no extra dependency.
    let id = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish() as u32;
    id.max(1)
}

impl Machine {
    /// `now` anchors the monotonic clock and `epoch_ms` is the wall clock at
    /// that instant, so frame timestamps are deterministic for a given input.
    pub fn new(config: Config, now: Instant, epoch_ms: u64) -> Self {
        let audio_session_id = config.audio_session_id.unwrap_or_else(random_session_id);
        Self {
            config,
            state: State::Created,
            audio_session_id,
            epoch_ms,
            started: now,
            deadline: None,
            next_ping: None,
            ping_id: 0,
            muted: false,
            subscribed_muted: false,
            offer: None,
            levels: std::collections::BTreeMap::new(),
            speaking: Vec::new(),
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// The earliest instant at which a `Tick` does something, or `None`. The
    /// caller sleeps until then (or until I/O); there is nothing to poll.
    pub fn next_deadline(&self) -> Option<Instant> {
        match (self.deadline, self.next_ping) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn handle(&mut self, now: Instant, input: Input) -> Vec<Output> {
        let mut out = Vec::new();
        if matches!(self.state, State::Left | State::Failed) {
            return out;
        }
        match input {
            Input::Opened => self.on_opened(now, &mut out),
            Input::Frame(bytes) => self.on_frame(now, &bytes, &mut out),
            Input::Tick => self.on_tick(now, &mut out),
            Input::LocalOffer { sdp } => self.on_offer(now, sdp, &mut out),
            Input::SetMuted(muted) => self.on_mute(muted, now, &mut out),
            Input::Leave => self.on_leave(now, &mut out),
            Input::Closed => self.on_closed(&mut out),
            Input::ClosedNormally => self.on_closed_normally(&mut out),
        }
        out
    }

    fn send(
        &self,
        now: Instant,
        ty: Type,
        fill: impl FnOnce(&mut SdkSignalFrame),
        out: &mut Vec<Output>,
    ) {
        let elapsed = now.saturating_duration_since(self.started).as_millis() as u64;
        let mut frame = SdkSignalFrame {
            timestamp_ms: self.epoch_ms.saturating_add(elapsed),
            r#type: ty as i32,
            ..Default::default()
        };
        fill(&mut frame);
        out.push(Output::Send(encode(&frame)));
    }

    fn fail(&mut self, reason: &str, out: &mut Vec<Output>) {
        self.state = State::Failed;
        self.deadline = None;
        self.next_ping = None;
        out.push(Output::Event(Event::Failed(reason.to_string())));
    }

    fn on_opened(&mut self, now: Instant, out: &mut Vec<Output>) {
        if self.state != State::Created {
            return;
        }
        let c = &self.config;
        let details = SdkClientDetails {
            app_name: non_empty(&c.app_name),
            app_version: non_empty(&c.app_version),
            platform_name: non_empty(&c.platform_name),
            os_name: non_empty(&c.os_name),
            os_version: non_empty(&c.os_version),
            client_source: Some("chime_signal".into()),
            ..Default::default()
        };
        let session = u64::from(self.audio_session_id);
        self.send(
            now,
            Type::Join,
            |f| {
                f.join = Some(SdkJoinFrame {
                    protocol_version: Some(2),
                    flags: Some(SdkJoinFlags::HasStreamUpdate as u32),
                    client_details: Some(details),
                    audio_session_id: Some(session),
                    wants_compressed_sdp: Some(false),
                    ..Default::default()
                });
            },
            out,
        );
        self.state = State::AwaitJoinAck;
        self.deadline = Some(now + HANDSHAKE_TIMEOUT);
        self.next_ping = Some(now + PING_INTERVAL);
    }

    fn on_tick(&mut self, now: Instant, out: &mut Vec<Output>) {
        if let Some(deadline) = self.deadline
            && now >= deadline
        {
            match self.state {
                State::AwaitJoinAck => self.fail("join ack timed out", out),
                State::AwaitIndex => self.fail("index timed out", out),
                State::AwaitSubscribeAck => self.fail("subscribe ack timed out", out),
                State::Leaving => {
                    self.state = State::Left;
                    self.deadline = None;
                    self.next_ping = None;
                    out.push(Output::Event(Event::Left));
                }
                _ => self.deadline = None,
            }
            return;
        }
        if let Some(due) = self.next_ping
            && now >= due
            && self.state != State::Leaving
        {
            self.ping_id = self.ping_id.wrapping_add(1);
            let id = self.ping_id;
            self.send(
                now,
                Type::PingPong,
                |f| {
                    f.ping_pong = Some(SdkPingPongFrame {
                        r#type: SdkPingPongType::Ping as i32,
                        ping_id: id,
                    });
                },
                out,
            );
            self.next_ping = Some(now + PING_INTERVAL);
        }
    }

    fn on_frame(&mut self, now: Instant, bytes: &[u8], out: &mut Vec<Output>) {
        if self.state == State::Created {
            return;
        }
        // A frame we cannot parse is dropped: a newer server may send types
        // this proto does not know, and that must not end the call.
        let Ok(frame) = decode(bytes) else { return };
        if let Some(err) = &frame.error {
            out.push(Output::Event(Event::Error {
                status: err.status.unwrap_or(0),
                description: err.description.clone().unwrap_or_default(),
            }));
            if !matches!(self.state, State::Joined | State::Leaving) {
                self.fail("server error during join", out);
            }
            return;
        }
        let Ok(ty) = Type::try_from(frame.r#type) else {
            return;
        };
        match ty {
            Type::PingPong => {
                if let Some(p) = &frame.ping_pong
                    && p.r#type == SdkPingPongType::Ping as i32
                {
                    let id = p.ping_id;
                    self.send(
                        now,
                        Type::PingPong,
                        |f| {
                            f.ping_pong = Some(SdkPingPongFrame {
                                r#type: SdkPingPongType::Pong as i32,
                                ping_id: id,
                            });
                        },
                        out,
                    );
                }
            }
            Type::JoinAck if self.state == State::AwaitJoinAck => {
                let creds = frame.joinack.and_then(|j| j.turn_credentials);
                if let Some(t) = creds {
                    out.push(Output::Event(Event::TurnCredentials(TurnCredentials {
                        username: Secret::new(t.username.unwrap_or_default()),
                        password: Secret::new(t.password.unwrap_or_default()),
                        ttl_seconds: t.ttl.unwrap_or(0),
                        uris: t.uris,
                    })));
                }
                self.state = State::AwaitIndex;
                self.deadline = Some(now + HANDSHAKE_TIMEOUT);
            }
            Type::Index
                if matches!(
                    self.state,
                    State::AwaitIndex
                        | State::AwaitOffer
                        | State::AwaitSubscribeAck
                        | State::Joined
                ) =>
            {
                let n = frame.index.and_then(|i| i.num_participants).unwrap_or(0);
                out.push(Output::Event(Event::Index {
                    num_participants: n,
                }));
                if self.state == State::AwaitIndex {
                    self.state = State::AwaitOffer;
                    self.deadline = None;
                    if let Some(sdp) = self.offer.take() {
                        self.subscribe(now, sdp, out);
                    }
                }
            }
            Type::SubscribeAck if self.state == State::AwaitSubscribeAck => {
                match frame.suback.and_then(|s| s.sdp_answer) {
                    Some(sdp) => {
                        out.push(Output::Event(Event::RemoteAnswer { sdp }));
                        self.state = State::Joined;
                        self.deadline = None;
                        if self.muted != self.subscribed_muted {
                            self.send_mute(now, out);
                        }
                    }
                    None => self.fail("subscribe ack carried no SDP answer", out),
                }
            }
            Type::AudioStreamIdInfo if self.state == State::Joined => {
                let streams = frame
                    .audio_stream_id_info
                    .map(|i| i.streams)
                    .unwrap_or_default();
                let people = streams
                    .into_iter()
                    .map(|s| Participant {
                        attendee_id: s.attendee_id.unwrap_or_default(),
                        external_user_id: s.external_user_id.unwrap_or_default(),
                        muted: s.muted.unwrap_or(false),
                        dropped: s.dropped.unwrap_or(false),
                        stream_id: s.audio_stream_id.unwrap_or(0),
                    })
                    .collect::<Vec<_>>();
                // A dropped stream stops speaking and is forgotten.
                for p in people.iter().filter(|p| p.dropped) {
                    self.levels.remove(&p.stream_id);
                }
                out.push(Output::Event(Event::Participants(people)));
                self.update_speaking(now, out);
            }
            Type::AudioMetadata if self.state == State::Joined => {
                let states = frame
                    .audio_metadata
                    .map(|m| m.attendee_states)
                    .unwrap_or_default();
                for st in states {
                    let Some(stream) = st.audio_stream_id else {
                        continue;
                    };
                    if !self.levels.contains_key(&stream)
                        && self.levels.len() >= MAX_TRACKED_STREAMS
                    {
                        continue;
                    }
                    let l = self.levels.entry(stream).or_insert(Level {
                        attenuation: u32::MAX,
                        muted: false,
                        speaking: false,
                        loud_at: now,
                    });
                    // A state without a volume keeps the last one.
                    if let Some(v) = st.volume {
                        l.attenuation = v;
                    }
                    if let Some(m) = st.muted {
                        l.muted = m;
                    }
                }
                self.update_speaking(now, out);
            }
            Type::LeaveAck if self.state == State::Leaving => {
                self.state = State::Left;
                self.deadline = None;
                self.next_ping = None;
                out.push(Output::Event(Event::Left));
            }
            _ => {}
        }
    }

    /// Re-judges every tracked stream at `now` and reports the speaking set
    /// when it changed. Muted stops at once; a quiet level stops only after
    /// [`SPEAKING_HOLD`]; the middle band keeps the state.
    fn update_speaking(&mut self, now: Instant, out: &mut Vec<Output>) {
        for l in self.levels.values_mut() {
            if l.muted {
                l.speaking = false;
            } else if l.attenuation <= SPEAKING_MAX_ATTENUATION {
                l.speaking = true;
                l.loud_at = now;
            } else if l.speaking
                && l.attenuation >= QUIET_MIN_ATTENUATION
                && now.saturating_duration_since(l.loud_at) >= SPEAKING_HOLD
            {
                l.speaking = false;
            }
        }
        let now_speaking: Vec<u32> = self
            .levels
            .iter()
            .filter(|(_, l)| l.speaking)
            .map(|(s, _)| *s)
            .collect();
        if now_speaking != self.speaking {
            self.speaking = now_speaking.clone();
            out.push(Output::Event(Event::Speaking(now_speaking)));
        }
    }

    fn on_offer(&mut self, now: Instant, sdp: String, out: &mut Vec<Output>) {
        match self.state {
            State::AwaitOffer => self.subscribe(now, sdp, out),
            State::AwaitJoinAck | State::AwaitIndex => self.offer = Some(sdp),
            _ => {}
        }
    }

    fn subscribe(&mut self, now: Instant, sdp: String, out: &mut Vec<Output>) {
        let muted = self.muted;
        let host = non_empty(&self.config.audio_host);
        // Like the Chime JS SDK: a normal join (`audio_checkin = false`) with an
        // audio host declares the one audio send stream.
        let send_streams = if host.is_some() {
            vec![SdkStreamDescriptor {
                stream_id: Some(1),
                framerate: Some(15),
                max_bitrate_kbps: Some(600),
                track_label: Some("AmazonChimeExpressAudio".into()),
                group_id: Some(1),
                avg_bitrate_bps: Some(400_000),
                attendee_id: Some(self.config.attendee_id.clone()),
                media_type: Some(SdkStreamMediaType::Audio as i32),
                ..Default::default()
            }]
        } else {
            Vec::new()
        };
        self.send(
            now,
            Type::Subscribe,
            |f| {
                f.sub = Some(SdkSubscribeFrame {
                    duplex: Some(SdkStreamServiceType::Rx as i32),
                    sdp_offer: Some(sdp),
                    audio_host: host,
                    audio_checkin: Some(false),
                    send_streams,
                    audio_muted: Some(muted),
                    ..Default::default()
                });
            },
            out,
        );
        self.subscribed_muted = muted;
        self.state = State::AwaitSubscribeAck;
        self.deadline = Some(now + HANDSHAKE_TIMEOUT);
    }

    fn on_mute(&mut self, muted: bool, now: Instant, out: &mut Vec<Output>) {
        self.muted = muted;
        // Before the subscribe the value rides in `audio_muted`; while its ack
        // is pending it is sent once the ack arrives; afterwards it is a frame.
        if self.state == State::Joined {
            self.send_mute(now, out);
        }
    }

    fn send_mute(&mut self, now: Instant, out: &mut Vec<Output>) {
        let muted = self.muted;
        self.send(
            now,
            Type::AudioControl,
            |f| {
                f.audio_control = Some(SdkAudioControlFrame { muted: Some(muted) });
            },
            out,
        );
        self.subscribed_muted = muted;
    }

    fn on_leave(&mut self, now: Instant, out: &mut Vec<Output>) {
        match self.state {
            State::Created => {
                self.state = State::Left;
                out.push(Output::Event(Event::Left));
            }
            State::Leaving => {}
            _ => {
                self.send(now, Type::Leave, |f| f.leave = Some(SdkLeaveFrame {}), out);
                self.state = State::Leaving;
                self.deadline = Some(now + LEAVE_TIMEOUT);
                self.next_ping = None;
            }
        }
    }

    fn on_closed_normally(&mut self, out: &mut Vec<Output>) {
        match self.state {
            State::Leaving | State::Joined => {
                self.state = State::Left;
                self.deadline = None;
                self.next_ping = None;
                out.push(Output::Event(Event::Left));
            }
            _ => self.fail("connection closed", out),
        }
    }

    fn on_closed(&mut self, out: &mut Vec<Output>) {
        match self.state {
            State::Leaving => {
                self.state = State::Left;
                self.deadline = None;
                out.push(Output::Event(Event::Left));
            }
            _ => self.fail("connection closed", out),
        }
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(test)]
mod tests;
