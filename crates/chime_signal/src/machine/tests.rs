use super::*;
use crate::codec::{decode, encode};
use crate::proto::sdk_signal_frame::Type;

fn config() -> Config {
    Config {
        app_name: "app".into(),
        app_version: "1.0".into(),
        platform_name: "desktop".into(),
        os_name: "os".into(),
        os_version: "1".into(),
        audio_host: "audio.example.test:3478".into(),
        audio_session_id: Some(777),
        attendee_id: "att-self".into(),
    }
}

fn frame(ty: Type) -> SdkSignalFrame {
    SdkSignalFrame {
        timestamp_ms: 1,
        r#type: ty as i32,
        ..Default::default()
    }
}

fn joinack() -> Vec<u8> {
    let mut f = frame(Type::JoinAck);
    f.joinack = Some(SdkJoinAckFrame {
        turn_credentials: Some(SdkTurnCredentials {
            username: Some("turn-user".into()),
            password: Some("turn-pass".into()),
            ttl: Some(300),
            uris: vec!["turn:relay.example.test:443".into()],
        }),
        ..Default::default()
    });
    encode(&f)
}

fn index(n: u32) -> Vec<u8> {
    let mut f = frame(Type::Index);
    f.index = Some(SdkIndexFrame {
        num_participants: Some(n),
        ..Default::default()
    });
    encode(&f)
}

fn suback(sdp: Option<&str>) -> Vec<u8> {
    let mut f = frame(Type::SubscribeAck);
    f.suback = Some(SdkSubscribeAckFrame {
        sdp_answer: sdp.map(Into::into),
        ..Default::default()
    });
    encode(&f)
}

fn ping(id: u32, ty: SdkPingPongType) -> Vec<u8> {
    let mut f = frame(Type::PingPong);
    f.ping_pong = Some(SdkPingPongFrame {
        r#type: ty as i32,
        ping_id: id,
    });
    encode(&f)
}

fn error_frame(status: u32, description: &str) -> Vec<u8> {
    let mut f = frame(Type::Notification);
    f.error = Some(SdkErrorFrame {
        status: Some(status),
        description: Some(description.into()),
    });
    encode(&f)
}

struct Rig {
    m: Machine,
    t0: Instant,
}

impl Rig {
    fn new() -> Self {
        let t0 = Instant::now();
        Self {
            m: Machine::new(config(), t0, 1_700_000_000_000),
            t0,
        }
    }

    fn at(&self, secs_ms: u64) -> Instant {
        self.t0 + Duration::from_millis(secs_ms)
    }

    fn feed(&mut self, ms: u64, input: Input) -> Vec<Output> {
        let now = self.at(ms);
        self.m.handle(now, input)
    }

    fn frame(&mut self, ms: u64, bytes: Vec<u8>) -> Vec<Output> {
        self.feed(ms, Input::Frame(bytes))
    }

    /// Drives Opened, joinack, index, offer, suback; returns at Joined.
    fn join(&mut self) {
        self.feed(0, Input::Opened);
        self.frame(10, joinack());
        self.frame(20, index(2));
        self.feed(
            30,
            Input::LocalOffer {
                sdp: "offer".into(),
            },
        );
        self.frame(40, suback(Some("answer")));
        assert_eq!(self.m.state(), State::Joined);
    }
}

fn sent(out: &[Output]) -> Vec<SdkSignalFrame> {
    out.iter()
        .filter_map(|o| match o {
            Output::Send(b) => Some(decode(b).expect("machine sends valid frames")),
            _ => None,
        })
        .collect()
}

fn events(out: &[Output]) -> Vec<&Event> {
    out.iter()
        .filter_map(|o| match o {
            Output::Event(e) => Some(e),
            _ => None,
        })
        .collect()
}

fn failed(out: &[Output]) -> bool {
    events(out).iter().any(|e| matches!(e, Event::Failed(_)))
}

#[test]
fn opened_sends_the_join_frame() {
    let mut r = Rig::new();
    let out = r.feed(0, Input::Opened);
    let frames = sent(&out);
    assert_eq!(frames.len(), 1);
    let f = &frames[0];
    assert_eq!(f.r#type, Type::Join as i32);
    assert_eq!(f.timestamp_ms, 1_700_000_000_000);
    let j = f.join.as_ref().unwrap();
    assert_eq!(j.protocol_version, Some(2));
    assert_eq!(j.flags, Some(SdkJoinFlags::HasStreamUpdate as u32));
    assert_eq!(j.audio_session_id, Some(777));
    assert_eq!(j.wants_compressed_sdp, Some(false));
    let d = j.client_details.as_ref().unwrap();
    assert_eq!(d.app_name.as_deref(), Some("app"));
    assert_eq!(d.platform_name.as_deref(), Some("desktop"));
    assert_eq!(r.m.state(), State::AwaitJoinAck);
}

#[test]
fn join_uses_a_random_nonzero_session_id_by_default() {
    let t0 = Instant::now();
    let mut ids = Vec::new();
    for _ in 0..4 {
        let mut m = Machine::new(Config::default(), t0, 0);
        let out = m.handle(t0, Input::Opened);
        let id = sent(&out)[0]
            .join
            .as_ref()
            .unwrap()
            .audio_session_id
            .unwrap();
        assert_ne!(id, 0);
        ids.push(id);
    }
    ids.sort_unstable();
    ids.dedup();
    assert!(ids.len() > 1, "ids should differ between sessions");
}

#[test]
fn full_sequence_offer_after_index() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);

    let out = r.frame(10, joinack());
    match events(&out).as_slice() {
        [Event::TurnCredentials(c)] => {
            assert_eq!(c.username.expose(), "turn-user");
            assert_eq!(c.password.expose(), "turn-pass");
            assert_eq!(c.ttl_seconds, 300);
            assert_eq!(c.uris, vec!["turn:relay.example.test:443".to_string()]);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(r.m.state(), State::AwaitIndex);

    let out = r.frame(20, index(2));
    assert_eq!(
        events(&out),
        vec![&Event::Index {
            num_participants: 2
        }]
    );
    assert!(sent(&out).is_empty(), "no subscribe before the offer");
    assert_eq!(r.m.state(), State::AwaitOffer);

    let out = r.feed(
        30,
        Input::LocalOffer {
            sdp: "offer-sdp".into(),
        },
    );
    let frames = sent(&out);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].r#type, Type::Subscribe as i32);
    let s = frames[0].sub.as_ref().unwrap();
    assert_eq!(s.duplex, Some(SdkStreamServiceType::Rx as i32));
    assert_eq!(s.sdp_offer.as_deref(), Some("offer-sdp"));
    assert_eq!(s.audio_host.as_deref(), Some("audio.example.test:3478"));
    assert_eq!(s.audio_checkin, Some(false));
    assert_eq!(s.send_streams.len(), 1);
    let d = &s.send_streams[0];
    assert_eq!(d.media_type, Some(SdkStreamMediaType::Audio as i32));
    assert_eq!(d.track_label.as_deref(), Some("AmazonChimeExpressAudio"));
    assert_eq!(d.attendee_id.as_deref(), Some("att-self"));
    assert_eq!(d.stream_id, Some(1));
    assert_eq!(d.group_id, Some(1));
    assert_eq!(d.framerate, Some(15));
    assert_eq!(d.max_bitrate_kbps, Some(600));
    assert_eq!(d.avg_bitrate_bps, Some(400_000));
    assert_eq!(s.audio_muted, Some(false));
    assert!(s.compressed_sdp_offer.is_none());
    assert_eq!(r.m.state(), State::AwaitSubscribeAck);

    let out = r.frame(40, suback(Some("answer-sdp")));
    assert_eq!(
        events(&out),
        vec![&Event::RemoteAnswer {
            sdp: "answer-sdp".into()
        }]
    );
    assert_eq!(r.m.state(), State::Joined);
}

#[test]
fn offer_before_index_is_held_until_index() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    r.frame(10, joinack());
    let out = r.feed(
        15,
        Input::LocalOffer {
            sdp: "early".into(),
        },
    );
    assert!(out.is_empty());
    let out = r.frame(20, index(1));
    let frames = sent(&out);
    assert_eq!(frames.len(), 1);
    assert_eq!(
        frames[0].sub.as_ref().unwrap().sdp_offer.as_deref(),
        Some("early")
    );
    assert_eq!(r.m.state(), State::AwaitSubscribeAck);
}

#[test]
fn server_ping_is_answered_with_the_same_id_in_every_open_state() {
    let mut r = Rig::new();
    assert!(
        r.frame(0, ping(1, SdkPingPongType::Ping)).is_empty(),
        "not opened yet"
    );
    r.feed(0, Input::Opened);
    for (ms, id) in [(1, 41), (2, 42)] {
        let out = r.frame(ms, ping(id, SdkPingPongType::Ping));
        let frames = sent(&out);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].r#type, Type::PingPong as i32);
        let p = frames[0].ping_pong.as_ref().unwrap();
        assert_eq!(p.r#type, SdkPingPongType::Pong as i32);
        assert_eq!(p.ping_id, id);
    }
    r.frame(10, joinack());
    r.frame(20, index(1));
    r.feed(30, Input::LocalOffer { sdp: "o".into() });
    r.frame(40, suback(Some("a")));
    let out = r.frame(50, ping(4_000_000_000, SdkPingPongType::Ping));
    assert_eq!(
        sent(&out)[0].ping_pong.as_ref().unwrap().ping_id,
        4_000_000_000
    );
    // A pong from the server needs no answer.
    assert!(r.frame(60, ping(1, SdkPingPongType::Pong)).is_empty());
}

#[test]
fn client_pings_every_ten_seconds_from_tick() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    assert_eq!(
        r.m.next_deadline(),
        Some(r.at(10_000)),
        "join ack and ping both at 10 s"
    );
    r.frame(10, joinack());
    r.frame(20, index(1));
    r.feed(30, Input::LocalOffer { sdp: "o".into() });
    r.frame(40, suback(Some("a")));
    assert_eq!(r.m.next_deadline(), Some(r.at(10_000)));
    assert!(r.feed(9_999, Input::Tick).is_empty());
    let out = r.feed(10_000, Input::Tick);
    let p = sent(&out)[0].ping_pong.unwrap();
    assert_eq!((p.r#type, p.ping_id), (SdkPingPongType::Ping as i32, 1));
    assert_eq!(r.m.next_deadline(), Some(r.at(20_000)));
    let out = r.feed(20_500, Input::Tick);
    assert_eq!(sent(&out)[0].ping_pong.as_ref().unwrap().ping_id, 2);
    assert_eq!(r.m.next_deadline(), Some(r.at(30_500)));
}

#[test]
fn join_ack_must_arrive_within_ten_seconds() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    assert!(!failed(&r.feed(9_999, Input::Tick)));
    let out = r.feed(10_000, Input::Tick);
    assert!(failed(&out));
    assert_eq!(r.m.state(), State::Failed);
    assert_eq!(r.m.next_deadline(), None);
}

#[test]
fn index_must_arrive_within_ten_seconds_of_the_join_ack() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    r.frame(5_000, joinack());
    assert!(!failed(&r.feed(14_999, Input::Tick)));
    assert!(failed(&r.feed(15_000, Input::Tick)));
}

#[test]
fn suback_must_arrive_within_ten_seconds_of_subscribe() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    r.frame(10, joinack());
    r.frame(20, index(1));
    r.feed(3_000, Input::LocalOffer { sdp: "o".into() });
    assert!(!failed(&r.feed(12_999, Input::Tick)));
    assert!(failed(&r.feed(13_000, Input::Tick)));
    assert_eq!(r.m.state(), State::Failed);
}

#[test]
fn no_deadline_before_the_socket_opens_or_while_waiting_for_the_offer() {
    let mut r = Rig::new();
    assert_eq!(r.m.next_deadline(), None);
    r.feed(0, Input::Opened);
    r.frame(10, joinack());
    r.frame(20, index(1));
    // Waiting for the caller's offer: only the ping deadline remains.
    assert_eq!(r.m.next_deadline(), Some(r.at(10_000)));
}

#[test]
fn handshake_error_frame_reports_error_then_fails() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    let out = r.frame(5, error_frame(403, "forbidden"));
    let ev = events(&out);
    assert_eq!(
        ev[0],
        &Event::Error {
            status: 403,
            description: "forbidden".into()
        }
    );
    assert!(matches!(ev[1], Event::Failed(_)));
    assert_eq!(r.m.state(), State::Failed);
}

#[test]
fn error_frame_after_join_is_reported_without_failing() {
    let mut r = Rig::new();
    r.join();
    let out = r.frame(100, error_frame(500, "hiccup"));
    assert_eq!(
        events(&out),
        vec![&Event::Error {
            status: 500,
            description: "hiccup".into()
        }]
    );
    assert_eq!(r.m.state(), State::Joined);
}

#[test]
fn suback_without_an_sdp_answer_fails() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    r.frame(10, joinack());
    r.frame(20, index(1));
    r.feed(30, Input::LocalOffer { sdp: "o".into() });
    assert!(failed(&r.frame(40, suback(None))));
}

#[test]
fn audio_stream_id_info_reports_participants() {
    let mut r = Rig::new();
    r.join();
    let mut f = frame(Type::AudioStreamIdInfo);
    f.audio_stream_id_info = Some(SdkAudioStreamIdInfoFrame {
        streams: vec![
            SdkAudioStreamIdInfo {
                audio_stream_id: Some(7),
                attendee_id: Some("att-1".into()),
                muted: Some(true),
                external_user_id: Some("ext-1".into()),
                dropped: None,
            },
            SdkAudioStreamIdInfo {
                audio_stream_id: Some(8),
                attendee_id: Some("att-2".into()),
                muted: Some(false),
                external_user_id: None,
                dropped: Some(true),
            },
        ],
    });
    let out = r.frame(100, encode(&f));
    assert_eq!(
        events(&out),
        vec![&Event::Participants(vec![
            Participant {
                attendee_id: "att-1".into(),
                external_user_id: "ext-1".into(),
                muted: true,
                dropped: false,
                stream_id: 7,
            },
            Participant {
                attendee_id: "att-2".into(),
                external_user_id: String::new(),
                muted: false,
                dropped: true,
                stream_id: 8,
            },
        ])]
    );
}

#[test]
fn index_frames_after_join_are_reported() {
    let mut r = Rig::new();
    r.join();
    let out = r.frame(100, index(5));
    assert_eq!(
        events(&out),
        vec![&Event::Index {
            num_participants: 5
        }]
    );
    assert!(sent(&out).is_empty());
}

#[test]
fn mute_when_joined_sends_an_audio_control_frame() {
    let mut r = Rig::new();
    r.join();
    for muted in [true, false] {
        let out = r.feed(100, Input::SetMuted(muted));
        let frames = sent(&out);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].r#type, Type::AudioControl as i32);
        assert_eq!(frames[0].audio_control.as_ref().unwrap().muted, Some(muted));
    }
}

#[test]
fn mute_before_subscribe_rides_in_audio_muted() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    assert!(r.feed(1, Input::SetMuted(true)).is_empty());
    r.frame(10, joinack());
    r.frame(20, index(1));
    let out = r.feed(30, Input::LocalOffer { sdp: "o".into() });
    assert_eq!(sent(&out)[0].sub.as_ref().unwrap().audio_muted, Some(true));
    let out = r.frame(40, suback(Some("a")));
    assert!(sent(&out).is_empty(), "already muted in the subscribe");
}

#[test]
fn mute_change_while_waiting_for_suback_is_sent_after_it() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    r.frame(10, joinack());
    r.frame(20, index(1));
    r.feed(30, Input::LocalOffer { sdp: "o".into() });
    assert!(r.feed(35, Input::SetMuted(true)).is_empty());
    let out = r.frame(40, suback(Some("a")));
    let frames = sent(&out);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].audio_control.as_ref().unwrap().muted, Some(true));
}

#[test]
fn leave_sends_leave_and_ends_on_leave_ack() {
    let mut r = Rig::new();
    r.join();
    let out = r.feed(100, Input::Leave);
    let frames = sent(&out);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].r#type, Type::Leave as i32);
    assert!(frames[0].leave.is_some());
    assert_eq!(r.m.state(), State::Leaving);
    assert_eq!(r.m.next_deadline(), Some(r.at(2_100)));

    let out = r.frame(150, encode(&frame(Type::LeaveAck)));
    assert_eq!(events(&out), vec![&Event::Left]);
    assert_eq!(r.m.state(), State::Left);
    assert_eq!(r.m.next_deadline(), None);
}

#[test]
fn leave_ends_on_socket_close() {
    let mut r = Rig::new();
    r.join();
    r.feed(100, Input::Leave);
    let out = r.feed(120, Input::Closed);
    assert_eq!(events(&out), vec![&Event::Left]);
    assert_eq!(r.m.state(), State::Left);
}

#[test]
fn leave_ends_after_two_seconds_without_an_ack() {
    let mut r = Rig::new();
    r.join();
    r.feed(100, Input::Leave);
    assert!(events(&r.feed(2_099, Input::Tick)).is_empty());
    assert_eq!(events(&r.feed(2_100, Input::Tick)), vec![&Event::Left]);
}

#[test]
fn leave_during_the_handshake_also_sends_leave() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    let out = r.feed(5, Input::Leave);
    assert_eq!(sent(&out)[0].r#type, Type::Leave as i32);
    assert_eq!(r.m.state(), State::Leaving);
}

#[test]
fn leave_before_the_socket_opened_just_ends() {
    let mut r = Rig::new();
    let out = r.feed(0, Input::Leave);
    assert_eq!(out, vec![Output::Event(Event::Left)]);
    assert_eq!(r.m.state(), State::Left);
}

#[test]
fn unexpected_close_fails_unless_already_ended() {
    let mut r = Rig::new();
    r.join();
    let out = r.feed(100, Input::Closed);
    assert!(failed(&out));
    assert_eq!(r.m.state(), State::Failed);
    assert!(r.feed(200, Input::Closed).is_empty());
}

#[test]
fn a_normal_close_while_joined_is_a_clean_end() {
    let mut r = Rig::new();
    r.join();
    let out = r.feed(100, Input::ClosedNormally);
    assert_eq!(events(&out), vec![&Event::Left]);
    assert_eq!(r.m.state(), State::Left);
}

#[test]
fn a_normal_close_during_the_handshake_still_fails() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    let out = r.feed(10, Input::ClosedNormally);
    assert!(failed(&out));
}

#[test]
fn malformed_frames_are_ignored() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    assert!(r.frame(1, vec![]).is_empty());
    assert!(r.frame(2, vec![0x05, 0xff, 0xff]).is_empty());
    assert_eq!(r.m.state(), State::AwaitJoinAck);
    assert!(!r.frame(10, joinack()).is_empty());
}

#[test]
fn out_of_order_frames_are_ignored() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    assert!(r.frame(1, suback(Some("a"))).is_empty());
    assert!(r.frame(2, index(1)).is_empty());
    assert_eq!(r.m.state(), State::AwaitJoinAck);
}

#[test]
fn nothing_happens_after_failed_or_left() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    r.feed(10_000, Input::Tick);
    assert_eq!(r.m.state(), State::Failed);
    assert!(r.frame(10_001, joinack()).is_empty());
    assert!(r.feed(10_002, Input::SetMuted(true)).is_empty());
    assert!(r.feed(10_003, Input::Leave).is_empty());
    assert!(r.feed(10_004, Input::Opened).is_empty());
}

#[test]
fn second_opened_is_ignored() {
    let mut r = Rig::new();
    r.feed(0, Input::Opened);
    assert!(r.feed(1, Input::Opened).is_empty());
}

#[test]
fn secrets_never_show_in_debug() {
    let c = TurnCredentials {
        username: Secret::new("turn-user"),
        password: Secret::new("turn-pass"),
        ttl_seconds: 1,
        uris: vec![],
    };
    for text in [
        format!("{c:?}"),
        format!("{:#?}", Event::TurnCredentials(c.clone())),
        format!("{:?}", c.password),
    ] {
        assert!(!text.contains("turn-user"), "{text}");
        assert!(!text.contains("turn-pass"), "{text}");
    }
}

fn metadata(states: &[(u32, u32, bool)]) -> Vec<u8> {
    let mut f = frame(Type::AudioMetadata);
    f.audio_metadata = Some(SdkAudioMetadataFrame {
        attendee_states: states
            .iter()
            .map(|&(stream, volume, muted)| SdkAudioAttendeeState {
                audio_stream_id: Some(stream),
                volume: Some(volume),
                muted: Some(muted),
                signal_strength: Some(2),
            })
            .collect(),
    });
    encode(&f)
}

fn speaking(out: &[Output]) -> Vec<Vec<u32>> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::Speaking(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn audio_metadata_reports_who_speaks_only_when_it_changes() {
    let mut r = Rig::new();
    r.join();
    // Stream 7 is loud (low attenuation), 8 is silent.
    let out = r.frame(100, metadata(&[(7, 10, false), (8, 127, false)]));
    assert_eq!(speaking(&out), vec![vec![7]]);
    // The same state again (the server repeats it often): nothing new.
    assert!(speaking(&r.frame(200, metadata(&[(7, 12, false), (8, 120, false)]))).is_empty());
    // A state that omits stream 7 keeps it speaking; 8 starts.
    let out = r.frame(300, metadata(&[(8, 5, false)]));
    assert_eq!(speaking(&out), vec![vec![7, 8]]);
    // A muted attendee stops at once, whatever its volume.
    let out = r.frame(400, metadata(&[(7, 5, true)]));
    assert_eq!(speaking(&out), vec![vec![8]]);
    // Before joining, metadata is ignored.
    let mut early = Rig::new();
    early.feed(0, Input::Opened);
    assert!(speaking(&early.frame(10, metadata(&[(7, 5, false)]))).is_empty());
}

#[test]
fn speaking_holds_through_short_pauses_and_ignores_the_middle_band() {
    let mut r = Rig::new();
    r.join();
    assert_eq!(
        speaking(&r.frame(1_000, metadata(&[(7, 10, false)]))),
        vec![vec![7]]
    );
    // A breath between words: quiet frames within the hold keep it on.
    assert!(speaking(&r.frame(1_200, metadata(&[(7, 127, false)]))).is_empty());
    assert!(speaking(&r.frame(1_600, metadata(&[(7, 127, false)]))).is_empty());
    // Past the hold (measured on frame arrival, no timer) it ends.
    let out = r.frame(
        1_000 + SPEAKING_HOLD.as_millis() as u64 + 1,
        metadata(&[(7, 127, false)]),
    );
    assert_eq!(speaking(&out), vec![Vec::<u32>::new()]);
    // Hysteresis: a level between the start and stop thresholds neither
    // starts a silent stream nor stops a speaking one.
    let mid = SPEAKING_MAX_ATTENUATION + 3;
    assert!(mid < QUIET_MIN_ATTENUATION);
    assert!(speaking(&r.frame(3_000, metadata(&[(8, mid, false)]))).is_empty());
    assert_eq!(
        speaking(&r.frame(3_100, metadata(&[(8, 10, false)]))),
        vec![vec![8]]
    );
    for t in [3_200, 4_000, 5_000] {
        assert!(speaking(&r.frame(t, metadata(&[(8, mid, false)]))).is_empty());
    }
}
