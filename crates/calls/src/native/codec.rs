//! Opus 20 ms frame encoder and decoder, mono 48 kHz.

use super::FRAME_SAMPLES;
use opus::{Application, Bitrate, Channels, Decoder, Encoder};

/// Longest Opus packet duration (120 ms) at 48 kHz: the decode scratch size.
pub const MAX_DECODE_SAMPLES: usize = 5760;

/// Largest Opus packet we produce.
const MAX_PACKET: usize = 400;

pub struct FrameEncoder {
    enc: Encoder,
}

impl FrameEncoder {
    pub fn new() -> Result<Self, opus::Error> {
        let mut enc = Encoder::new(super::SAMPLE_RATE, Channels::Mono, Application::Voip)?;
        enc.set_bitrate(Bitrate::Bits(32_000))?;
        enc.set_complexity(5)?;
        // Let the receiver recover a single lost packet from the next one.
        enc.set_inband_fec(true)?;
        enc.set_packet_loss_perc(10)?;
        Ok(Self { enc })
    }

    /// Encodes one [`FRAME_SAMPLES`] frame.
    pub fn encode(&mut self, pcm: &[f32]) -> Result<Vec<u8>, opus::Error> {
        debug_assert_eq!(pcm.len(), FRAME_SAMPLES);
        self.enc.encode_vec_float(pcm, MAX_PACKET)
    }
}

pub struct FrameDecoder {
    dec: Decoder,
}

impl FrameDecoder {
    pub fn new() -> Result<Self, opus::Error> {
        Ok(Self {
            dec: Decoder::new(super::SAMPLE_RATE, Channels::Mono)?,
        })
    }

    /// Decodes a payload into `out` (at least [`MAX_DECODE_SAMPLES`] long
    /// for arbitrary packets) and returns the sample count. An empty payload
    /// conceals one lost [`FRAME_SAMPLES`] frame.
    pub fn decode(&mut self, payload: &[u8], out: &mut [f32]) -> Result<usize, opus::Error> {
        if payload.is_empty() {
            return self.dec.decode_float(&[], &mut out[..FRAME_SAMPLES], false);
        }
        self.dec.decode_float(payload, out, false)
    }

    /// Recovers the [`FRAME_SAMPLES`] frame that preceded `next` from the
    /// forward error correction carried in `next`.
    pub fn decode_fec(&mut self, next: &[u8], out: &mut [f32]) -> Result<usize, opus::Error> {
        self.dec.decode_float(next, &mut out[..FRAME_SAMPLES], true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn tone(hz: f32, frames: usize) -> Vec<f32> {
        (0..frames * FRAME_SAMPLES)
            .map(|i| 0.3 * (2.0 * std::f32::consts::PI * hz * i as f32 / 48_000.0).sin())
            .collect()
    }

    /// Dominant frequency by correlating against a 5 Hz grid.
    fn dominant_hz(pcm: &[f32]) -> f32 {
        let mut best = (0.0f32, 0.0f32);
        let mut hz = 100.0;
        while hz < 2000.0 {
            let w = 2.0 * std::f32::consts::PI * hz / 48_000.0;
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, s) in pcm.iter().enumerate() {
                re += s * (w * i as f32).cos();
                im += s * (w * i as f32).sin();
            }
            let mag = re * re + im * im;
            if mag > best.1 {
                best = (hz, mag);
            }
            hz += 5.0;
        }
        best.0
    }

    #[test]
    fn tone_survives_encode_decode() {
        let mut enc = FrameEncoder::new().unwrap();
        let mut dec = FrameDecoder::new().unwrap();
        let pcm = tone(440.0, 25);
        let mut out = Vec::new();
        for frame in pcm.chunks(FRAME_SAMPLES) {
            let packet = enc.encode(frame).unwrap();
            assert!(!packet.is_empty() && packet.len() < MAX_PACKET);
            let mut buf = vec![0.0; MAX_DECODE_SAMPLES];
            let n = dec.decode(&packet, &mut buf).unwrap();
            assert_eq!(n, FRAME_SAMPLES);
            out.extend_from_slice(&buf[..n]);
        }
        let tail = &out[10 * FRAME_SAMPLES..];
        let hz = dominant_hz(tail);
        assert!((hz - 440.0).abs() <= 10.0, "dominant {hz}");
        let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
        assert!(rms > 0.1, "rms {rms}");
    }

    #[test]
    fn concealment_returns_a_full_frame() {
        let mut enc = FrameEncoder::new().unwrap();
        let mut dec = FrameDecoder::new().unwrap();
        let pcm = tone(440.0, 6);
        let mut buf = vec![0.0; MAX_DECODE_SAMPLES];
        for frame in pcm.chunks(FRAME_SAMPLES) {
            let packet = enc.encode(frame).unwrap();
            dec.decode(&packet, &mut buf).unwrap();
        }
        let n = dec.decode(&[], &mut buf).unwrap();
        assert_eq!(n, FRAME_SAMPLES);
        assert!(buf[..n].iter().any(|s| s.abs() > 0.01));
    }

    #[test]
    fn fec_recovers_a_frame_from_the_next_packet() {
        let mut enc = FrameEncoder::new().unwrap();
        let mut dec = FrameDecoder::new().unwrap();
        let pcm = tone(440.0, 8);
        let mut buf = vec![0.0; MAX_DECODE_SAMPLES];
        let packets: Vec<_> = pcm
            .chunks(FRAME_SAMPLES)
            .map(|f| enc.encode(f).unwrap())
            .collect();
        for p in &packets[..5] {
            dec.decode(p, &mut buf).unwrap();
        }
        // Frame 5 is lost; frame 6 carries its FEC.
        let n = dec.decode_fec(&packets[6], &mut buf).unwrap();
        assert_eq!(n, FRAME_SAMPLES);
    }
}
