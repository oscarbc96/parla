//! Frame codec: one prefix byte followed by a protobuf `SdkSignalFrame`.

use crate::proto::SdkSignalFrame;
use crate::proto::sdk_signal_frame::Type;
use prost::Message;

/// Prefix byte of frames this client sends.
pub const CLIENT_PREFIX: u8 = 0x05;
/// Prefix byte some servers use on frames they send.
pub const LEGACY_PREFIX: u8 = 0x02;
/// Largest frame (prefix included) accepted by [`decode`].
pub const MAX_FRAME_LEN: usize = 1024 * 1024;

/// Why a frame could not be decoded.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("empty frame")]
    Empty,
    #[error("frame of {0} bytes exceeds the {MAX_FRAME_LEN} byte cap")]
    TooLarge(usize),
    #[error("unknown frame prefix byte {0:#04x}")]
    BadPrefix(u8),
    #[error("malformed frame: {0}")]
    Malformed(String),
    #[error("unknown frame type {0}")]
    UnknownType(i32),
}

/// Serialises a frame with the `0x05` prefix.
pub fn encode(frame: &SdkSignalFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + frame.encoded_len());
    out.push(CLIENT_PREFIX);
    frame
        .encode(&mut out)
        .expect("a Vec grows, so encoding cannot run out of space");
    out
}

/// `prost` fills a missing proto2 `required` field with a default (the first
/// enum value for `type`), so presence of tags 1 and 2 is checked on the wire.
fn has_required_fields(mut body: &[u8]) -> bool {
    use prost::encoding::{DecodeContext, decode_key, skip_field};
    let (mut timestamp, mut kind) = (false, false);
    while !body.is_empty() {
        let Ok((tag, wire)) = decode_key(&mut body) else {
            return false;
        };
        match tag {
            1 => timestamp = true,
            2 => kind = true,
            _ => {}
        }
        if skip_field(wire, tag, &mut body, DecodeContext::default()).is_err() {
            return false;
        }
    }
    timestamp && kind
}

/// Parses a prefixed frame.
pub fn decode(bytes: &[u8]) -> Result<SdkSignalFrame, Error> {
    let (&prefix, body) = bytes.split_first().ok_or(Error::Empty)?;
    if bytes.len() > MAX_FRAME_LEN {
        return Err(Error::TooLarge(bytes.len()));
    }
    if prefix != CLIENT_PREFIX && prefix != LEGACY_PREFIX {
        return Err(Error::BadPrefix(prefix));
    }
    if !has_required_fields(body) {
        return Err(Error::Malformed("missing timestamp_ms or type".into()));
    }
    let frame = SdkSignalFrame::decode(body).map_err(|e| Error::Malformed(e.to_string()))?;
    if Type::try_from(frame.r#type).is_err() {
        return Err(Error::UnknownType(frame.r#type));
    }
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::*;

    fn frame(ty: Type) -> SdkSignalFrame {
        SdkSignalFrame {
            timestamp_ms: 1_700_000_000_000,
            r#type: ty as i32,
            ..Default::default()
        }
    }

    fn roundtrip(f: SdkSignalFrame) {
        let bytes = encode(&f);
        assert_eq!(bytes[0], 0x05);
        assert_eq!(decode(&bytes).unwrap(), f);
    }

    #[test]
    fn join_roundtrips() {
        let mut f = frame(Type::Join);
        f.join = Some(SdkJoinFrame {
            protocol_version: Some(2),
            flags: Some(SdkJoinFlags::HasStreamUpdate as u32),
            audio_session_id: Some(42),
            wants_compressed_sdp: Some(false),
            client_details: Some(SdkClientDetails {
                app_name: Some("app".into()),
                ..Default::default()
            }),
            ..Default::default()
        });
        roundtrip(f);
    }

    #[test]
    fn joinack_roundtrips() {
        let mut f = frame(Type::JoinAck);
        f.joinack = Some(SdkJoinAckFrame {
            turn_credentials: Some(SdkTurnCredentials {
                username: Some("u".into()),
                password: Some("p".into()),
                ttl: Some(300),
                uris: vec!["turn:example.test:443".into()],
            }),
            ..Default::default()
        });
        roundtrip(f);
    }

    #[test]
    fn index_roundtrips() {
        let mut f = frame(Type::Index);
        f.index = Some(SdkIndexFrame {
            num_participants: Some(3),
            ..Default::default()
        });
        roundtrip(f);
    }

    #[test]
    fn subscribe_roundtrips() {
        let mut f = frame(Type::Subscribe);
        f.sub = Some(SdkSubscribeFrame {
            duplex: Some(SdkStreamServiceType::Duplex as i32),
            sdp_offer: Some("v=0".into()),
            audio_host: Some("host.example.test".into()),
            audio_checkin: Some(true),
            audio_muted: Some(false),
            ..Default::default()
        });
        roundtrip(f);
    }

    #[test]
    fn suback_roundtrips() {
        let mut f = frame(Type::SubscribeAck);
        f.suback = Some(SdkSubscribeAckFrame {
            sdp_answer: Some("v=0 answer".into()),
            ..Default::default()
        });
        roundtrip(f);
    }

    #[test]
    fn audio_stream_id_info_roundtrips() {
        let mut f = frame(Type::AudioStreamIdInfo);
        f.audio_stream_id_info = Some(SdkAudioStreamIdInfoFrame {
            streams: vec![SdkAudioStreamIdInfo {
                audio_stream_id: Some(7),
                attendee_id: Some("a".into()),
                muted: Some(true),
                external_user_id: Some("e".into()),
                dropped: Some(false),
            }],
        });
        roundtrip(f);
    }

    #[test]
    fn ping_pong_roundtrips() {
        for ty in [SdkPingPongType::Ping, SdkPingPongType::Pong] {
            let mut f = frame(Type::PingPong);
            f.ping_pong = Some(SdkPingPongFrame {
                r#type: ty as i32,
                ping_id: 9,
            });
            roundtrip(f);
        }
    }

    #[test]
    fn leave_and_ack_roundtrip() {
        let mut f = frame(Type::Leave);
        f.leave = Some(SdkLeaveFrame {});
        roundtrip(f);
        let mut f = frame(Type::LeaveAck);
        f.leave_ack = Some(SdkLeaveAckFrame {});
        roundtrip(f);
    }

    #[test]
    fn error_roundtrips() {
        let mut f = frame(Type::Notification);
        f.error = Some(SdkErrorFrame {
            status: Some(403),
            description: Some("nope".into()),
        });
        roundtrip(f);
    }

    #[test]
    fn decodes_a_0x02_prefixed_server_frame() {
        let f = frame(Type::Index);
        let mut bytes = encode(&f);
        bytes[0] = 0x02;
        assert_eq!(decode(&bytes).unwrap(), f);
    }

    #[test]
    fn rejects_empty_and_bad_prefix() {
        assert_eq!(decode(&[]), Err(Error::Empty));
        let mut bytes = encode(&frame(Type::Leave));
        bytes[0] = 0x07;
        assert_eq!(decode(&bytes), Err(Error::BadPrefix(0x07)));
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        assert!(decode(&[0x05]).is_err());
        assert!(decode(&[0x05, 0xff, 0xff, 0xff]).is_err());
        assert!(decode(&[0x02, 0x08]).is_err());
        let mut seed = 0x1234_5678_u32;
        for len in 1..200 {
            let mut buf = vec![0x05];
            for _ in 0..len {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                buf.push((seed >> 24) as u8);
            }
            let _ = decode(&buf);
        }
    }

    #[test]
    fn rejects_unknown_frame_type() {
        let mut f = frame(Type::Leave);
        f.r#type = 999;
        assert_eq!(decode(&encode(&f)), Err(Error::UnknownType(999)));
    }

    #[test]
    fn rejects_frames_over_the_cap() {
        let mut bytes = vec![0x05];
        bytes.resize(MAX_FRAME_LEN + 1, 0);
        assert_eq!(decode(&bytes), Err(Error::TooLarge(MAX_FRAME_LEN + 1)));
    }
}
